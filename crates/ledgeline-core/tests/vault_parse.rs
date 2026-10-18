//! Parsing through a [`Vault`] (`parse_journal_in`): the storage seam the
//! desktop and any future host share.
//!
//! Every case here runs on a [`MemVault`], which has no local path at all — so
//! each one is also proof that the parser needs nothing but the trait.

use ledgeline_core::parse::{ParseError, parse_journal_in};
use ledgeline_core::vault::{Availability, MemVault, Revision, Vault, VaultPath};
use std::collections::HashMap;

const TXN_2026: &str = "2026-01-02 coffee\n    expenses:food  $3.00\n    assets:cash\n";
const TXN_2025: &str = "2025-06-01 rent\n    expenses:rent  $900.00\n    assets:bank\n";

fn vp(path: &str) -> VaultPath {
    VaultPath::new(path).expect("valid vault path")
}

fn no_overrides() -> HashMap<VaultPath, String> {
    HashMap::new()
}

fn books() -> MemVault {
    MemVault::new()
        .with_file(
            "books/main.journal",
            "account assets:cash\ninclude 2026/2026.journal\ninclude 2025.journal\n",
        )
        .with_file("books/2026/2026.journal", TXN_2026)
        .with_file("books/2025.journal", TXN_2025)
}

fn err_text(result: Result<ledgeline_core::parse::ParsedJournal, ParseError>) -> String {
    result.expect_err("the parse must fail").to_string()
}

#[test]
fn a_journal_parses_entirely_through_the_vault_with_one_revision_per_source() {
    let vault = books();
    let parsed =
        parse_journal_in(&vault, &vp("books/main.journal"), &no_overrides()).expect("parses");
    assert_eq!(parsed.journal.transactions.len(), 2);

    // Keys are the vault's: the source name is the main file's key, and every
    // transaction is recorded under its own file's key.
    let key = |p: &str| vault.key(&vp(p));
    assert_eq!(
        parsed.journal.source_name,
        key("books/main.journal").to_string_lossy()
    );
    assert_eq!(
        parsed.journal.source_files,
        vec![
            key("books/main.journal"),
            key("books/2026/2026.journal"),
            key("books/2025.journal"),
        ]
    );
    assert_eq!(
        parsed.journal.transactions[0].source_file,
        key("books/2026/2026.journal")
    );

    // One source per file, first-read order, each with exactly the bytes parsed
    // and the revision the vault reports for them now.
    let paths: Vec<&str> = parsed.sources.iter().map(|s| s.path.as_str()).collect();
    assert_eq!(
        paths,
        vec![
            "books/main.journal",
            "books/2026/2026.journal",
            "books/2025.journal"
        ]
    );
    for source in &parsed.sources {
        let (text, revision) = source.read.as_ref().expect("read from storage");
        let (bytes, now) = vault.read_pinned(&source.path, None).expect("readable");
        assert_eq!(text.as_bytes(), bytes.as_slice());
        assert!(revision.matches(&now), "{}", source.path);
    }
}

#[test]
fn overrides_are_parsed_instead_of_read_and_carry_no_revision() {
    let vault = books();
    let overrides = HashMap::from([(vp("books/2025.journal"), format!("{TXN_2025}\n{TXN_2025}"))]);
    let parsed = parse_journal_in(&vault, &vp("books/main.journal"), &overrides).expect("parses");
    assert_eq!(parsed.journal.transactions.len(), 3);
    let overridden = parsed
        .sources
        .iter()
        .find(|s| s.path == vp("books/2025.journal"))
        .expect("listed");
    assert!(overridden.read.is_none(), "an override was never read");

    // The main file itself can be overridden, as the editor's validate does.
    let overrides = HashMap::from([(vp("books/main.journal"), TXN_2026.to_string())]);
    let parsed = parse_journal_in(&vault, &vp("books/main.journal"), &overrides).expect("parses");
    assert_eq!(parsed.journal.transactions.len(), 1);
    assert_eq!(parsed.sources.len(), 1);
    assert!(parsed.sources[0].read.is_none());
}

#[test]
fn a_not_downloaded_main_file_fails_fast_and_says_why() {
    let vault = books();
    vault.set_availability("books/main.journal", Availability::NotDownloaded);
    let err = err_text(parse_journal_in(
        &vault,
        &vp("books/main.journal"),
        &no_overrides(),
    ));
    assert!(err.contains("could not read"), "{err}");
    assert!(err.contains("not downloaded"), "{err}");
}

#[test]
fn a_not_downloaded_include_fails_at_its_directive() {
    let vault = books();
    vault.set_availability("books/2025.journal", Availability::NotDownloaded);
    let err = err_text(parse_journal_in(
        &vault,
        &vp("books/main.journal"),
        &no_overrides(),
    ));
    assert!(err.contains("not downloaded"), "{err}");
    assert!(err.contains("include 2025.journal"), "{err}");
    // And once it is downloaded, the same journal opens.
    vault.request_download(&[vp("books/2025.journal")]);
    parse_journal_in(&vault, &vp("books/main.journal"), &no_overrides()).expect("parses");
}

#[test]
fn an_include_cannot_leave_the_main_journal_s_directory() {
    let vault = books().with_file("secret.journal", "this-must-not-be-echoed\n");
    for written in [
        "../secret.journal",
        "/etc/passwd",
        "2026/../../secret.journal",
    ] {
        vault.insert_file("books/main.journal", format!("include {written}\n"));
        let err = err_text(parse_journal_in(
            &vault,
            &vp("books/main.journal"),
            &no_overrides(),
        ));
        assert!(
            err.contains("resolves outside the journal directory"),
            "{written}: {err}"
        );
        assert!(
            err.contains(written),
            "quotes the include as written: {err}"
        );
        assert!(!err.contains("this-must-not-be-echoed"), "{err}");
    }
}

#[test]
fn a_symlink_out_of_the_tree_is_refused_by_its_resolved_path() {
    let vault = books().with_file("secret.journal", "this-must-not-be-echoed\n");
    vault.insert_symlink("books/link.journal", "../secret.journal");
    vault.insert_file("books/main.journal", "include link.journal\n");
    let err = err_text(parse_journal_in(
        &vault,
        &vp("books/main.journal"),
        &no_overrides(),
    ));
    assert!(
        err.contains("resolves outside the journal directory"),
        "{err}"
    );
    assert!(
        !err.contains("secret.journal"),
        "the target is not named: {err}"
    );
}

#[test]
fn a_symlink_inside_the_tree_is_followed_and_recorded_as_its_target() {
    let vault = books();
    vault.insert_symlink("books/current.journal", "2026/2026.journal");
    vault.insert_file("books/main.journal", "include current.journal\n");
    let parsed =
        parse_journal_in(&vault, &vp("books/main.journal"), &no_overrides()).expect("parses");
    assert_eq!(
        parsed.journal.transactions[0].source_file,
        vault.key(&vp("books/2026/2026.journal"))
    );
}

#[test]
fn hidden_entries_and_cycles_are_refused_before_any_read() {
    let vault = books().with_file("books/.git/config", "[core]\n");
    vault.insert_file("books/main.journal", "include .git/config\n");
    let err = err_text(parse_journal_in(
        &vault,
        &vp("books/main.journal"),
        &no_overrides(),
    ));
    assert!(err.contains("hidden entry '.git'"), "{err}");
    assert!(!err.contains("[core]"), "{err}");

    vault.insert_file("books/main.journal", "include a.journal\n");
    vault.insert_file("books/a.journal", "include main.journal\n");
    let err = err_text(parse_journal_in(
        &vault,
        &vp("books/main.journal"),
        &no_overrides(),
    ));
    assert!(err.contains("forms a cycle"), "{err}");
}

#[test]
fn a_missing_include_is_a_read_error_naming_the_file() {
    let vault = books();
    vault.insert_file("books/main.journal", "include nope.journal\n");
    let err = err_text(parse_journal_in(
        &vault,
        &vp("books/main.journal"),
        &no_overrides(),
    ));
    assert!(!err.contains("resolves outside"), "{err}");
    assert!(err.contains("nope.journal"), "{err}");
}

#[test]
fn a_change_after_the_parse_is_visible_to_the_returned_revision() {
    let vault = books();
    let parsed =
        parse_journal_in(&vault, &vp("books/main.journal"), &no_overrides()).expect("parses");
    let (_, revision): &(String, Revision) = parsed.sources[1].read.as_ref().expect("read");
    vault.modify_in_place("books/2026/2026.journal", TXN_2025);
    let (_, now) = vault
        .read_pinned(&vp("books/2026/2026.journal"), None)
        .expect("readable");
    assert!(
        !revision.matches(&now),
        "the parse's revision describes what was parsed, not what is there now"
    );
}

#[test]
fn the_text_only_entry_point_includes_nothing_for_a_placeholder_name() {
    // `parse_journal` with a name that is not a real file: no directory to
    // resolve against, so an include resolves nowhere — never against the
    // process's working directory.
    let err = ledgeline_core::parse_journal("include Cargo.toml\n", "placeholder.journal")
        .expect_err("nothing to include")
        .to_string();
    assert!(err.contains("Cargo.toml"), "{err}");
    assert!(!err.contains("[package]"), "{err}");
}
