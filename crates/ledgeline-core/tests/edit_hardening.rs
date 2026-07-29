//! Hardening regression tests for the journal write path.
//!
//! These lock in three fixes that are invisible to the functional edit tests
//! because they concern file *metadata* and *malformed input* rather than
//! journal text:
//!
//! - **SEC-3** — saving must not widen the journal's permissions. The temp file
//!   used to be created with `File::create` (`0666 & ~umask`) and renamed over
//!   the target, so a journal correctly kept at `0600` came back `0644` under the
//!   common `umask 022`.
//! - **SEC-8** — the temp file must be created exclusively, and a symlinked
//!   journal must survive the rename as a symlink.
//! - **SEC-2 item 4** — an absurd `places` must not panic or be written.
//!
//! Every test is Unix-only (it asserts on `st_mode`) and cleans its scratch
//! directory up through a drop guard, so a panicking test leaves nothing behind.
//!
//! Note on umask: the fix makes the result umask-independent, but the *bug* was
//! not. `owner_only` fails under a permissive umask and `group_readable` fails
//! under a restrictive one, so the pair catches a regression whatever umask the
//! test runner happens to have.

#![cfg(unix)]

use ledgeline_core::decimal::Dec;
use ledgeline_core::edit::{EditError, InsertPosition, JournalEditor, atomic_write};
use ledgeline_core::model::{
    AccountName, Amount, AmountStyle, Commodity, CommoditySide, Posting, PostingType, SourcePos,
    Status, Tindex, Transaction,
};
use std::fs::Permissions;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const JOURNAL: &str = "\
2026-01-01 * Opening
    assets:cash  $100.00
    equity:opening
";

// ---------------------------------------------------------------------------
// Scratch directories + helpers
// ---------------------------------------------------------------------------

static SEQ: AtomicU64 = AtomicU64::new(0);

/// A unique scratch directory that removes itself on drop — including when the
/// test panics, so a failing assertion never leaves files in the temp dir.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "ledgeline-hardening-{name}-{}-{seq}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The permission bits of `path`, symlinks followed.
fn mode_of(path: &Path) -> u32 {
    std::fs::metadata(path).expect("stat").permissions().mode() & 0o7777
}

fn write_journal(path: &Path, mode: u32) {
    std::fs::write(path, JOURNAL).expect("write journal");
    std::fs::set_permissions(path, Permissions::from_mode(mode)).expect("chmod journal");
}

fn dollars(mantissa: i128, places: u32) -> Amount {
    Amount {
        commodity: Commodity("$".into()),
        quantity: Dec::new(mantissa, places),
        style: AmountStyle {
            side: CommoditySide::Left,
            spaced: false,
            decimal_mark: Some('.'),
            digit_groups: None,
            precision: 2,
        },
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

fn txn_with(amount: Amount) -> Transaction {
    Transaction {
        index: Tindex(0),
        date: "2026-02-01".into(),
        date2: None,
        status: Status::Cleared,
        code: String::new(),
        description: "Coffee".into(),
        comment: String::new(),
        preceding_comment: String::new(),
        tags: vec![],
        postings: vec![
            leg("expenses:coffee", Some(amount)),
            leg("assets:cash", None),
        ],
        source_file: PathBuf::new(),
        source_span: (
            SourcePos { line: 1, column: 1 },
            SourcePos { line: 1, column: 1 },
        ),
    }
}

/// Drive exactly one committed edit and save through the public editor API —
/// the same path the HTTP `POST /api/transactions` handler takes.
fn add_one_and_save(journal: &Path) {
    let mut editor = JournalEditor::open(journal.to_path_buf()).expect("open journal");
    editor
        .add_transaction(&txn_with(dollars(500, 2)), InsertPosition::Append)
        .expect("add transaction");
    editor.save().expect("save");
}

// ---------------------------------------------------------------------------
// SEC-3 — saving must not widen the journal's permissions
// ---------------------------------------------------------------------------

#[test]
fn saving_keeps_an_owner_only_journal_owner_only() {
    let scratch = Scratch::new("sec3-600");
    let journal = scratch.path().join("perm.journal");
    write_journal(&journal, 0o600);

    add_one_and_save(&journal);

    assert_eq!(
        mode_of(&journal),
        0o600,
        "a journal kept at 0600 must not become world-readable on save"
    );
}

#[test]
fn saving_preserves_a_group_readable_journal_exactly() {
    let scratch = Scratch::new("sec3-640");
    let journal = scratch.path().join("perm.journal");
    write_journal(&journal, 0o640);

    add_one_and_save(&journal);

    assert_eq!(
        mode_of(&journal),
        0o640,
        "the target's mode is preserved, not replaced by a hardcoded default"
    );
}

#[test]
fn saving_repeatedly_does_not_drift_the_mode() {
    let scratch = Scratch::new("sec3-drift");
    let journal = scratch.path().join("perm.journal");
    write_journal(&journal, 0o600);

    for _ in 0..3 {
        add_one_and_save(&journal);
        assert_eq!(mode_of(&journal), 0o600, "mode must be stable across saves");
    }
}

#[test]
fn atomic_write_creates_a_missing_file_owner_only() {
    let scratch = Scratch::new("sec3-new");
    let path = scratch.path().join("fresh.journal");

    atomic_write(&path, b"fresh\n").expect("write new file");

    assert_eq!(
        mode_of(&path),
        0o600,
        "a journal that does not exist yet defaults closed, not umask-derived"
    );
    assert_eq!(std::fs::read_to_string(&path).expect("read"), "fresh\n");
}

#[test]
fn atomic_write_leaves_no_temp_files_behind() {
    let scratch = Scratch::new("sec3-temp");
    let journal = scratch.path().join("perm.journal");
    write_journal(&journal, 0o600);

    add_one_and_save(&journal);

    let leftovers: Vec<PathBuf> = std::fs::read_dir(scratch.path())
        .expect("read scratch dir")
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.to_string_lossy().contains(".ledgeline-"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "temp files left behind: {leftovers:?}"
    );
}

// ---------------------------------------------------------------------------
// SEC-8 — symlink handling
// ---------------------------------------------------------------------------

#[test]
fn atomic_write_follows_a_symlink_instead_of_replacing_it() {
    let scratch = Scratch::new("sec8-link");
    let real = scratch.path().join("real.journal");
    let link = scratch.path().join("link.journal");
    write_journal(&real, 0o640);
    std::os::unix::fs::symlink(&real, &link).expect("create symlink");

    atomic_write(&link, b"rewritten\n").expect("write through the symlink");

    assert!(
        std::fs::symlink_metadata(&link)
            .expect("lstat link")
            .file_type()
            .is_symlink(),
        "the rename must not replace the symlink with a regular file"
    );
    assert_eq!(
        std::fs::read_to_string(&real).expect("read real"),
        "rewritten\n",
        "the write must land on the file the link points at"
    );
    assert_eq!(
        mode_of(&real),
        0o640,
        "the link target's mode is preserved too"
    );
}

#[test]
fn atomic_write_through_a_relative_symlink_resolves_against_the_link_dir() {
    let scratch = Scratch::new("sec8-rel");
    let real = scratch.path().join("real.journal");
    let link = scratch.path().join("link.journal");
    write_journal(&real, 0o600);
    // A RELATIVE target must be resolved against the link's own directory, not
    // the process working directory.
    std::os::unix::fs::symlink("real.journal", &link).expect("create relative symlink");

    atomic_write(&link, b"rewritten\n").expect("write through the relative symlink");

    assert!(
        std::fs::symlink_metadata(&link)
            .expect("lstat link")
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        std::fs::read_to_string(&real).expect("read real"),
        "rewritten\n"
    );
}

#[test]
fn atomic_write_through_a_dangling_symlink_creates_the_named_target() {
    let scratch = Scratch::new("sec8-dangling");
    let target = scratch.path().join("not-yet.journal");
    let link = scratch.path().join("link.journal");
    std::os::unix::fs::symlink(&target, &link).expect("create dangling symlink");

    atomic_write(&link, b"created\n").expect("write through the dangling symlink");

    assert!(
        std::fs::symlink_metadata(&link)
            .expect("lstat link")
            .file_type()
            .is_symlink(),
        "a dangling link is resolved, not overwritten"
    );
    assert_eq!(
        std::fs::read_to_string(&target).expect("read target"),
        "created\n"
    );
    assert_eq!(
        mode_of(&target),
        0o600,
        "a newly created target is owner-only"
    );
}

#[test]
fn atomic_write_terminates_on_a_symlink_loop() {
    let scratch = Scratch::new("sec8-loop");
    let a = scratch.path().join("a.journal");
    let b = scratch.path().join("b.journal");
    std::os::unix::fs::symlink(&b, &a).expect("link a -> b");
    std::os::unix::fs::symlink(&a, &b).expect("link b -> a");

    // The property under test is TERMINATION: symlink resolution is capped, so
    // this returns instead of spinning forever. A loop names no real file, so
    // the write necessarily breaks it by landing on the last hop — the same
    // outcome the unhardened code produced, and the only one available.
    atomic_write(&a, b"looped\n").expect("a capped resolution still writes");

    assert_eq!(
        std::fs::read_to_string(&a).expect("read through a"),
        "looped\n",
        "the loop is broken by a real file, reachable through the original path"
    );
}

// ---------------------------------------------------------------------------
// SEC-2 item 4 — an absurd `places` must not panic, and must not be written
// ---------------------------------------------------------------------------

#[test]
fn an_over_precise_amount_is_refused_and_the_journal_is_untouched() {
    let scratch = Scratch::new("sec2-places");
    let journal = scratch.path().join("perm.journal");
    write_journal(&journal, 0o600);

    let mut editor = JournalEditor::open(journal.clone()).expect("open journal");
    // `places = 65_535` used to panic inside `render_dec` ("Formatting argument
    // out of range"), taking down the request with no HTTP response at all.
    let result = editor.add_transaction(&txn_with(dollars(500, 65_535)), InsertPosition::Append);

    assert!(
        matches!(result, Err(EditError::RoundTripMismatch)),
        "expected the reparse guard to reject a clamped amount, got {result:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&journal).expect("read journal"),
        JOURNAL,
        "a refused edit must leave the file byte-identical"
    );
}

#[test]
fn a_high_but_sane_precision_amount_still_round_trips() {
    let scratch = Scratch::new("sec2-sane");
    let journal = scratch.path().join("perm.journal");
    write_journal(&journal, 0o600);

    let mut editor = JournalEditor::open(journal.clone()).expect("open journal");
    // Well inside MAX_RENDER_PLACES and inside what the parser stores, so the
    // clamp must not disturb it.
    editor
        .add_transaction(
            &txn_with(dollars(5_000_000_000, 10)),
            InsertPosition::Append,
        )
        .expect("a 10-place amount is ordinary and must be accepted");
    editor.save().expect("save");

    assert!(
        std::fs::read_to_string(&journal)
            .expect("read journal")
            .contains("0.5000000000"),
        "the exact 10-place amount must reach the file"
    );
}
