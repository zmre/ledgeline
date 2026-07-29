//! Write-path guard regressions: DL-3 (external-change detection), DL-4 (the
//! post-edit verification), the elided-amount round trip, and the DL-6
//! insert-position / formatting fixes.
//!
//! Unix-only: the DL-3 case restores an mtime with `touch -r`, which is how the
//! code review reproduced it and what every mtime-preserving copy tool does.

#![cfg(unix)]

use ledgeline_core::edit::{EditError, InsertPosition, JournalEditor};
use ledgeline_core::model::{Status, Tindex, Transaction};
use ledgeline_core::parse::check_transaction_balances;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

static SEQ: AtomicU64 = AtomicU64::new(0);

/// A unique scratch directory that removes itself on drop, including on panic.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "ledgeline-guards-{name}-{}-{seq}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        Self(dir)
    }

    /// Write `text` to `name` inside the scratch directory and return its path.
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

fn mtime_of(path: &Path) -> SystemTime {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .expect("mtime")
}

/// Run a command, requiring it to succeed.
fn run(program: &str, args: &[&std::ffi::OsStr]) {
    let status = std::process::Command::new(program)
        .args(args)
        .status()
        .unwrap_or_else(|e| panic!("spawn {program}: {e}"));
    assert!(status.success(), "{program} failed: {status}");
}

const THREE: &str = "\
2026-01-01 * A
    expenses:a  $1.00
    assets:cash

2026-01-02 * B
    expenses:b  $2.00
    assets:cash

2026-01-03 * C
    expenses:c  $3.00
    assets:cash
";

// ---------------------------------------------------------------------------
// DL-3 — the mtime fast path silently clobbered an external edit
// ---------------------------------------------------------------------------

/// An external write whose mtime is then restored must still be detected.
///
/// `file_changed_externally` used to return "unchanged" WITHOUT reading the file
/// whenever the mtime matched the load-time one. Any mtime-preserving write
/// defeats that — `cp -p`, `rsync -t`, `tar -x`, a snapshot restore, an
/// mtime-preserving editor, or two writes inside one tick on a coarse-grained
/// filesystem. Here `touch -r` restores it, exactly as the review reproduced it,
/// and `save()` used to succeed and wipe the external transaction with no error.
#[test]
fn an_mtime_restoring_external_write_is_still_refused() {
    let scratch = Scratch::new("dl3");
    let journal = scratch.write("main.journal", THREE);

    let mut editor = JournalEditor::open(&journal).expect("open journal");
    let loaded_mtime = mtime_of(&journal);

    // A reference file carrying the load-time mtime, the way a backup would.
    let stamp = scratch.0.join("stamp");
    run(
        "cp",
        &["-p".as_ref(), journal.as_os_str(), stamp.as_os_str()],
    );

    // Someone else adds a transaction...
    let external =
        format!("{THREE}\n2026-01-04 * External\n    expenses:d  $4.00\n    assets:cash\n");
    std::fs::write(&journal, &external).expect("external write");
    // ...and the mtime is put back, which is the whole point of the case.
    run(
        "touch",
        &["-r".as_ref(), stamp.as_os_str(), journal.as_os_str()],
    );
    assert_eq!(
        mtime_of(&journal),
        loaded_mtime,
        "the repro is only meaningful if the mtime really was restored"
    );

    editor.delete_transaction(Tindex(2)).expect("delete B");
    let result = editor.save();

    assert!(
        matches!(result, Err(EditError::ExternalChange)),
        "save must refuse an mtime-restored external write, got {result:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&journal).expect("read back"),
        external,
        "the external transaction must survive untouched"
    );
}

/// The mirror case: a touch that leaves the BYTES identical is not an external
/// change, so the content hash must let the save through. Losing this would
/// make `touch journal` (or any no-op rewrite) block every subsequent edit.
#[test]
fn a_content_preserving_touch_still_saves() {
    let scratch = Scratch::new("dl3-touch");
    let journal = scratch.write("main.journal", THREE);

    let mut editor = JournalEditor::open(&journal).expect("open journal");
    run("touch", &[journal.as_os_str()]);

    editor.delete_transaction(Tindex(2)).expect("delete B");
    editor
        .save()
        .expect("a mtime-only touch is not a content change");
    assert!(
        !std::fs::read_to_string(&journal)
            .expect("read back")
            .contains("2026-01-02"),
        "the delete must have reached the file"
    );
}

// ---------------------------------------------------------------------------
// DL-4 — the post-edit verification beyond the transaction count
// ---------------------------------------------------------------------------

/// A journal that ALREADY contains an unbalanced transaction has to stay
/// editable. The balance half of the guard compares against the pre-edit state
/// for exactly this reason: demanding a wholly balanced journal would let one
/// bad row freeze the whole file, including the edit that would fix it.
#[test]
fn a_pre_existing_imbalance_does_not_block_an_unrelated_edit() {
    let scratch = Scratch::new("dl4-preexisting");
    let text = "\
2026-01-01 * Wrong
    expenses:a   $1.00
    assets:cash  $-2.00

2026-01-02 * Fine
    expenses:b  $2.00
    assets:cash
";
    let journal = scratch.write("main.journal", text);
    let mut editor = JournalEditor::open(&journal).expect("open journal");

    // The imbalance is real and detected...
    assert_eq!(
        check_transaction_balances(editor.journal())
            .expect("balance check")
            .len(),
        1,
        "the fixture must actually be unbalanced"
    );

    // ...and an unrelated edit still goes through.
    editor
        .set_status(Tindex(2), Status::Pending)
        .expect("an unrelated edit must not be blocked by a pre-existing imbalance");
    editor.save().expect("save");
    assert!(
        std::fs::read_to_string(&journal)
            .expect("read back")
            .contains("2026-01-02 ! Fine")
    );
}

/// Every edit must leave every OTHER transaction byte-identical. Deleting the
/// middle of three is the case the count check could not distinguish from
/// deleting the wrong lines.
#[test]
fn untouched_transactions_stay_byte_identical_through_a_delete() {
    let scratch = Scratch::new("dl4-identity");
    let journal = scratch.write("main.journal", THREE);
    let mut editor = JournalEditor::open(&journal).expect("open journal");

    let a = editor.transaction_source(Tindex(1)).expect("A");
    let c = editor.transaction_source(Tindex(3)).expect("C");

    editor.delete_transaction(Tindex(2)).expect("delete B");

    assert_eq!(editor.transaction_source(Tindex(1)).as_deref(), Some(&*a));
    assert_eq!(editor.transaction_source(Tindex(2)).as_deref(), Some(&*c));
}

// ---------------------------------------------------------------------------
// Elided amounts
// ---------------------------------------------------------------------------

/// A leg the user wrote blank must come back blank.
///
/// The SPA reads a transaction (with the parser's inferred amount filled in) and
/// PUTs every posting back with an explicit amount, so a full replace used to
/// harden `assets:bank:checking` into `$-1800.00`. That is not cosmetic: once
/// the inferred leg is written out, it can no longer disagree with the others,
/// and hledger's own imbalance detection is permanently disabled for that
/// transaction.
#[test]
fn an_elided_leg_survives_a_full_replace() {
    let scratch = Scratch::new("elision");
    let text = "\
2026-01-01 * Landlord | rent
    expenses:housing:rent  $1800.00
    assets:bank:checking
";
    let journal = scratch.write("main.journal", text);
    let mut editor = JournalEditor::open(&journal).expect("open journal");

    // What the edit API sends back: the parsed transaction, every amount
    // explicit, with one field changed.
    let mut edited: Transaction = editor.journal().transactions[0].clone();
    assert_eq!(
        edited.postings[1].amounts.len(),
        1,
        "the parser fills the elided leg in, which is what the SPA round-trips"
    );
    edited.description = "Landlord | rent (Feb)".into();

    editor
        .replace_transaction(Tindex(1), &edited)
        .expect("replace");
    editor.save().expect("save");

    assert_eq!(
        std::fs::read_to_string(&journal).expect("read back"),
        "2026-01-01 * Landlord | rent (Feb)\n    \
         expenses:housing:rent  $1800.00\n    assets:bank:checking\n",
        "the blank leg must stay blank"
    );
}

/// An elided leg carrying a balance assertion keeps BOTH: the leg stays blank
/// and the `= $99.00` reconciliation anchor survives. The formatter used to
/// render an amount-less posting as the bare account, silently dropping the
/// assertion — the one case where nothing else on the line records the balance.
///
/// The fixture is a journal `hledger 1.52` loads cleanly (opening balance
/// $1899.00, less $1800.00 rent, asserts $99.00), so the assertion is real
/// rather than decorative.
#[test]
fn an_elided_leg_keeps_its_balance_assertion() {
    let scratch = Scratch::new("elision-assertion");
    let text = "\
2026-01-01 * Opening
    assets:bank:checking  $1899.00
    equity:opening

2026-01-02 * Rent
    expenses:rent  $1800.00
    assets:bank:checking    = $99.00
";
    let journal = scratch.write("main.journal", text);
    let mut editor = JournalEditor::open(&journal).expect("open journal");

    let mut edited: Transaction = editor.journal().transactions[1].clone();
    assert!(
        edited.postings[1].balance_assertion.is_some(),
        "the fixture must carry an assertion"
    );
    assert_eq!(
        edited.postings[1].amounts.len(),
        1,
        "the parser infers the leg, which is what the SPA round-trips back"
    );
    edited.description = "Rent (Feb)".into();
    editor
        .replace_transaction(Tindex(2), &edited)
        .expect("replace");

    let after = editor.text();
    assert!(
        after.contains("assets:bank:checking  = $99.00"),
        "the elided leg must keep its assertion, got:\n{after}"
    );
    assert!(
        !after.contains("$-1800.00"),
        "the leg must stay elided, got:\n{after}"
    );
}

/// An amount the user DID write stays written — the elision restore must not
/// blank a leg that was explicit in the source.
#[test]
fn an_explicit_leg_is_not_blanked() {
    let scratch = Scratch::new("elision-explicit");
    let text = "\
2026-01-01 * Rent
    expenses:rent         $1800.00
    assets:bank:checking  $-1800.00
";
    let journal = scratch.write("main.journal", text);
    let mut editor = JournalEditor::open(&journal).expect("open journal");

    let mut edited: Transaction = editor.journal().transactions[0].clone();
    edited.description = "Rent (Feb)".into();
    editor
        .replace_transaction(Tindex(1), &edited)
        .expect("replace");

    assert!(
        editor.text().contains("assets:bank:checking  $-1800.00"),
        "an explicitly written amount must be preserved, got:\n{}",
        editor.text()
    );
}

// ---------------------------------------------------------------------------
// DL-6 — insert position, CRLF, and header restyling
// ---------------------------------------------------------------------------

/// A date-ordered add whose neighbours live in different `include`d files must
/// land in the file that holds its own period, not blindly in the predecessor's.
#[test]
fn date_ordered_add_lands_in_the_file_matching_its_period() {
    let scratch = Scratch::new("dl6-include");
    scratch.write(
        "2025.journal",
        "2025-11-01 * Late 2025\n    expenses:d  $40.00\n    assets:bank\n",
    );
    scratch.write(
        "2026.journal",
        "2026-03-01 * Spring 2026\n    expenses:e  $50.00\n    assets:bank\n",
    );
    let main = scratch.write(
        "main.journal",
        "include 2025.journal\ninclude 2026.journal\n",
    );

    let mut editor = JournalEditor::open(&main).expect("open journal");
    let january = new_txn(&editor, "2026-01-05");
    editor
        .add_transaction(&january, InsertPosition::DateOrdered)
        .expect("add");
    editor.save().expect("save");

    let y2025 = std::fs::read_to_string(scratch.0.join("2025.journal")).expect("read 2025");
    let y2026 = std::fs::read_to_string(scratch.0.join("2026.journal")).expect("read 2026");
    assert!(
        !y2025.contains("2026-01-05"),
        "a 2026 row must not be written into 2025.journal:\n{y2025}"
    );
    assert!(
        y2026.contains("2026-01-05"),
        "a 2026 row belongs in 2026.journal:\n{y2026}"
    );

    // The mirror case must still follow the PREDECESSOR: 2025-12-15 belongs in
    // 2025.journal even though its successor lives in 2026.journal.
    let mut editor = JournalEditor::open(&main).expect("reopen");
    let december = new_txn(&editor, "2025-12-15");
    editor
        .add_transaction(&december, InsertPosition::DateOrdered)
        .expect("add december");
    editor.save().expect("save december");
    assert!(
        std::fs::read_to_string(scratch.0.join("2025.journal"))
            .expect("read 2025")
            .contains("2025-12-15"),
        "a December 2025 row belongs in 2025.journal"
    );
}

/// A CRLF journal must keep CRLF endings; a mixed-terminator file is what makes
/// the next whitespace-normalising tool rewrite the whole thing.
#[test]
fn a_crlf_journal_gets_crlf_insertions() {
    let scratch = Scratch::new("dl6-crlf");
    let journal = scratch.write(
        "main.journal",
        "2026-01-01 * A\r\n    expenses:a  $1.00\r\n    assets:cash\r\n",
    );

    let mut editor = JournalEditor::open(&journal).expect("open journal");
    let february = new_txn(&editor, "2026-02-01");
    editor
        .add_transaction(&february, InsertPosition::Append)
        .expect("add");
    editor.save().expect("save");

    let after = std::fs::read_to_string(&journal).expect("read back");
    assert!(
        !after.replace("\r\n", "").contains('\n'),
        "every line must end \\r\\n, got {after:?}"
    );
    assert!(after.contains("2026-02-01"), "the row must be there");
}

/// `set_status` must change the status and nothing else — in particular it must
/// not restyle the user's own date.
#[test]
fn a_header_rewrite_preserves_the_users_date_style() {
    let scratch = Scratch::new("dl6-datestyle");
    let journal = scratch.write(
        "main.journal",
        "2026/01/01 * (42) Payee\n    expenses:a  $1.00\n    assets:cash\n",
    );

    let mut editor = JournalEditor::open(&journal).expect("open journal");
    editor
        .set_status(Tindex(1), Status::Pending)
        .expect("set status");

    assert_eq!(
        editor.text(),
        "2026/01/01 ! (42) Payee\n    expenses:a  $1.00\n    assets:cash\n",
        "only the status marker may change"
    );
}

/// A trailing indented comment inside a transaction body belongs to that
/// transaction, so a date-ordered insert after it must land BELOW the comment.
///
/// The review flagged this as broken because `insert_after` anchors on
/// `source_span`'s end, which used to stop at the last POSTING line. PARSE-7
/// changed that — `source_span` now runs past trailing in-body comment lines —
/// so this half is already fixed upstream. The tag is load-bearing (`hledger`
/// reads `subscription: false` from it and the subscriptions report keys off
/// it), so it is worth a test that says so out loud.
#[test]
fn an_insert_after_leaves_a_trailing_tag_comment_with_its_transaction() {
    let scratch = Scratch::new("dl6-trailing-comment");
    let journal = scratch.write(
        "main.journal",
        "2026-01-01 * Gym\n    \
         expenses:fitness  $30.00\n    \
         assets:cash\n    \
         ; subscription: false\n\n\
         2026-03-01 * Later\n    expenses:x  $1.00\n    assets:cash\n",
    );

    let mut editor = JournalEditor::open(&journal).expect("open journal");
    let february = new_txn(&editor, "2026-02-01");
    editor
        .add_transaction(&february, InsertPosition::DateOrdered)
        .expect("add");

    let after = editor.text();
    let tag = after.find("; subscription: false").expect("tag present");
    let new_row = after.find("2026-02-01").expect("new row present");
    assert!(
        tag < new_row,
        "the tag must stay inside the Gym transaction, above the new row:\n{after}"
    );

    // And it is still parsed as the Gym transaction's own tag.
    let gym = editor
        .journal()
        .transactions
        .iter()
        .find(|t| t.description == "Gym")
        .expect("Gym present");
    assert!(
        gym.postings.iter().any(|p| p
            .tags
            .iter()
            .any(|(k, v)| k == "subscription" && v == "false")),
        "the tag must still belong to Gym, got {:?}",
        gym.postings.iter().map(|p| &p.tags).collect::<Vec<_>>()
    );
}

/// A comment block written directly above a transaction describes THAT
/// transaction, so a date-ordered insert has to go above the whole block.
#[test]
fn an_insert_before_keeps_a_comment_with_its_transaction() {
    let scratch = Scratch::new("dl6-comment");
    let journal = scratch.write(
        "main.journal",
        "; journal preamble\n\n\
         ; the Berlin trip\n\
         2026-03-01 * Trip\n    expenses:travel  $10.00\n    assets:cash\n",
    );

    let mut editor = JournalEditor::open(&journal).expect("open journal");
    let earlier = new_txn(&editor, "2026-01-01");
    editor
        .add_transaction(&earlier, InsertPosition::DateOrdered)
        .expect("add");

    let after = editor.text();
    let note = after.find("; the Berlin trip").expect("note present");
    let new_row = after.find("2026-01-01").expect("new row present");
    assert!(
        new_row < note,
        "the new row must go ABOVE the note that describes the trip, got:\n{after}"
    );
    // The file's own preamble stays on top.
    assert!(
        after.starts_with("; journal preamble"),
        "the preamble must stay first, got:\n{after}"
    );
}

/// Build a balanced two-leg transaction dated `date`, reusing the styles the
/// open journal already parsed so no fixture has to restate them.
fn new_txn(editor: &JournalEditor, date: &str) -> Transaction {
    let mut txn = editor.journal().transactions[0].clone();
    txn.date = date.to_string();
    txn.date2 = None;
    txn.description = "New".into();
    txn.comment = String::new();
    txn.tags = vec![];
    txn
}
