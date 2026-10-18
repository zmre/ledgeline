//! The editor over a [`Vault`]: open, edit, save, and every way the bytes on
//! storage can stop being the bytes the editor read.
//!
//! All on a [`MemVault`], which has no local path — the same shape an iOS host
//! hands the editor — and which counts its reads, so "the parse's read is the
//! pin" is a measured property rather than a comment.

use ledgeline_core::model::Tindex;
use ledgeline_core::vault::{Availability, MemVault, Vault, VaultPath};
use ledgeline_core::{EditError, JournalEditor};
use std::sync::Arc;

const MAIN: &str = "account assets:cash\ninclude 2026.journal\ninclude prices.journal\n";
const TXNS: &str = "2026-01-02 coffee\n    expenses:food  $3.00\n    assets:cash\n\n\
                    2026-01-03 lunch\n    expenses:food  $12.00\n    assets:cash\n";
const PRICES: &str = "P 2026-01-01 AAPL $200.00\n";

fn vp(path: &str) -> VaultPath {
    VaultPath::new(path).expect("valid vault path")
}

fn books() -> Arc<MemVault> {
    Arc::new(
        MemVault::new()
            .with_file("books/main.journal", MAIN)
            .with_file("books/2026.journal", TXNS)
            .with_file("books/prices.journal", PRICES),
    )
}

fn open(vault: &Arc<MemVault>) -> JournalEditor {
    let shared: Arc<dyn Vault> = Arc::clone(vault) as Arc<dyn Vault>;
    JournalEditor::open_in(shared, vp("books/main.journal")).expect("opens")
}

#[test]
fn opening_reads_every_source_exactly_once() {
    let vault = books();
    let editor = open(&vault);
    assert_eq!(editor.transaction_count(), 2);
    // main + 2026 + prices: one read each. Before the seam, `load` re-read
    // every include that held a transaction to pin it, leaving a window in
    // which the pinned bytes need not be the parsed ones.
    assert_eq!(vault.read_count(), 3);
    assert_eq!(
        editor.path(),
        vault.key(&vp("books/main.journal")),
        "keys are the vault's"
    );
    assert!(editor.changed_sources().is_empty());
}

#[test]
fn an_edit_saves_only_the_file_it_touched_through_the_vault() {
    let vault = books();
    let mut editor = open(&vault);
    editor
        .set_description(Tindex(1), "espresso")
        .expect("edit applies");
    assert_eq!(editor.dirty_files().len(), 1);
    editor.save().expect("saves");
    let saved = vault.text("books/2026.journal").expect("present");
    assert!(saved.starts_with("2026-01-02 espresso\n"), "{saved}");
    assert!(saved.contains("2026-01-03 lunch"), "{saved}");
    assert_eq!(vault.text("books/main.journal").unwrap(), MAIN, "untouched");
    assert_eq!(vault.text("books/prices.journal").unwrap(), PRICES);
    // Our own write is not an external change, for the next save or for a poll.
    assert!(editor.changed_sources().is_empty());
    editor
        .delete_transaction(Tindex(2))
        .expect("delete applies");
    editor
        .save()
        .expect("a second save pins to the first's revision");
    assert!(!vault.text("books/2026.journal").unwrap().contains("lunch"));
}

#[test]
fn a_file_rewritten_in_place_behind_the_editor_is_never_overwritten() {
    let vault = books();
    let mut editor = open(&vault);
    editor.set_description(Tindex(1), "espresso").expect("edit");
    let external =
        format!("{TXNS}\n2026-01-04 external\n    expenses:food  $1.00\n    assets:cash\n");
    vault.modify_in_place("books/2026.journal", &external);
    assert!(matches!(editor.save(), Err(EditError::ExternalChange)));
    assert_eq!(vault.text("books/2026.journal").unwrap(), external);
}

#[test]
fn a_file_replaced_by_an_identical_copy_is_still_refused() {
    // Same bytes, new identity: another program saved by rename. The content
    // matches, but the editor pinned the file it read, not a file like it.
    let vault = books();
    let mut editor = open(&vault);
    editor.set_description(Tindex(1), "espresso").expect("edit");
    vault.modify_externally("books/2026.journal", TXNS);
    assert!(matches!(editor.save(), Err(EditError::ExternalChange)));
    assert_eq!(vault.text("books/2026.journal").unwrap(), TXNS);
}

#[test]
fn a_file_that_became_a_symlink_or_vanished_is_refused() {
    let vault = books();
    let mut editor = open(&vault);
    editor.set_description(Tindex(1), "espresso").expect("edit");
    vault.insert_file("books/elsewhere.journal", TXNS);
    vault.insert_symlink("books/2026.journal", "elsewhere.journal");
    assert!(matches!(editor.save(), Err(EditError::ExternalChange)));
    assert_eq!(vault.text("books/elsewhere.journal").unwrap(), TXNS);

    let vault = books();
    let mut editor = open(&vault);
    editor.set_description(Tindex(1), "espresso").expect("edit");
    vault
        .remove_file(&vp("books/2026.journal"))
        .expect("removed");
    assert!(editor.save().is_err(), "nothing to pin to");
    assert_eq!(vault.text("books/2026.journal"), None, "not recreated");
}

#[test]
fn a_not_downloaded_journal_fails_to_open_fast_and_opens_once_downloaded() {
    let vault = books();
    vault.set_availability("books/2026.journal", Availability::NotDownloaded);
    let shared: Arc<dyn Vault> = Arc::clone(&vault) as Arc<dyn Vault>;
    let err = JournalEditor::open_in(Arc::clone(&shared), vp("books/main.journal"))
        .err()
        .expect("cannot open a journal whose include is still in the cloud");
    assert!(err.to_string().contains("not downloaded"), "{err}");
    vault.request_download(&[vp("books/2026.journal")]);
    JournalEditor::open_in(shared, vp("books/main.journal")).expect("opens once local");
}

#[test]
fn a_file_evicted_to_the_cloud_before_save_is_not_written() {
    let vault = books();
    let mut editor = open(&vault);
    editor.set_description(Tindex(1), "espresso").expect("edit");
    vault.set_availability("books/2026.journal", Availability::NotDownloaded);
    let err = editor
        .save()
        .expect_err("cannot compare against bytes not here");
    match err {
        EditError::Io(io) => assert_eq!(io.kind(), std::io::ErrorKind::WouldBlock),
        other => panic!("expected a not-downloaded I/O error, got {other:?}"),
    }
    vault.request_download(&[vp("books/2026.journal")]);
    assert_eq!(vault.text("books/2026.journal").unwrap(), TXNS);
    editor.save().expect("saves once the file is local again");
}

#[test]
fn changed_sources_sees_every_file_including_directive_only_includes() {
    let vault = books();
    let editor = open(&vault);
    vault.modify_in_place("books/prices.journal", "P 2026-01-01 AAPL $201.00\n");
    assert_eq!(editor.changed_sources(), vec![vp("books/prices.journal")]);
    // An identical rewrite, or a touch, is not a change (DL-3).
    let vault = books();
    let editor = open(&vault);
    vault.modify_externally("books/main.journal", MAIN);
    assert!(editor.changed_sources().is_empty());
}

#[test]
fn reopen_rereads_through_the_same_vault_and_refuses_a_main_file_turned_link() {
    let vault = books();
    let editor = open(&vault);
    vault.modify_in_place(
        "books/2026.journal",
        format!("{TXNS}\n2026-01-04 more\n    expenses:food  $1.00\n    assets:cash\n"),
    );
    let reopened = editor.reopen().expect("reopens");
    assert_eq!(reopened.transaction_count(), 3);

    vault.insert_file("books/other.journal", "");
    vault.insert_symlink("books/main.journal", "other.journal");
    assert!(matches!(reopened.reopen(), Err(EditError::ExternalChange)));
}
