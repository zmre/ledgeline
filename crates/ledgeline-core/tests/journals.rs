//! Ranking the journal files an import could be written to, over the four
//! `fixtures/import/layouts/` trees.
//!
//! The trees are the anti-assumption fixtures. Between them they use
//! `main.journal` and `all.journal` as roots, split by year and by month, spell
//! months as words so that alphabetical order is *not* date order, and include a
//! `prices.journal` whose newest date is later than any transaction in the tree.
//! Every one of those defeats a plausible filename heuristic, and none of them
//! defeats reading the content.
//!
//! A test here that passes because a name was recognized is a failing test. The
//! last two cases exist for exactly that: they build a tree at run time in which
//! every name lies about what the file holds, and assert the ranking follows the
//! transactions.

mod common;

use ledgeline_core::journals::{JournalTarget, targets};
use ledgeline_core::parse_journal;
use std::path::{Path, PathBuf};

/// Parse a layout fixture from its root file.
fn layout(root: &str) -> Vec<JournalTarget> {
    let path = common::fixtures_dir().join("import/layouts").join(root);
    ranked(&path)
}

/// Parse the journal rooted at `path` and rank its files.
fn ranked(path: &Path) -> Vec<JournalTarget> {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{} readable: {e}", path.display()));
    let journal = parse_journal(&text, &path.to_string_lossy())
        .unwrap_or_else(|e| panic!("{} parses: {e}", path.display()));
    targets(&journal)
}

/// The ids, best-first — the whole answer, in one comparable shape.
fn ids(targets: &[JournalTarget]) -> Vec<&str> {
    targets.iter().map(|target| target.id.as_str()).collect()
}

fn find<'a>(targets: &'a [JournalTarget], id: &str) -> &'a JournalTarget {
    targets
        .iter()
        .find(|target| target.id == id)
        .unwrap_or_else(|| panic!("{id} should be listed; got {:?}", ids(targets)))
}

// ---------------------------------------------------------------------------
// The four layouts
// ---------------------------------------------------------------------------

#[test]
fn a_single_file_journal_offers_itself() {
    let targets = layout("single/main.journal");
    assert_eq!(ids(&targets), ["main.journal"]);
    let only = find(&targets, "main.journal");
    assert!(only.is_root);
    assert!(only.writable);
    assert_eq!(only.txn_count, 3);
    assert_eq!(only.last_txn_date.as_deref(), Some("2026-02-18"));
    assert_eq!(only.label, "main.journal");
}

#[test]
fn a_year_split_journal_ranks_the_newest_year_first() {
    let targets = layout("split-year/main.journal");
    assert_eq!(
        ids(&targets),
        [
            "2026/2026.journal",
            "2025/2025.journal",
            "main.journal",
            "accounts.journal",
            "prices.journal",
        ],
        "newest transaction first; the three files holding none rank last, in \
         the order the parse read them"
    );
    assert_eq!(
        find(&targets, "2026/2026.journal").last_txn_date.as_deref(),
        Some("2026-02-11")
    );
    assert_eq!(
        find(&targets, "2025/2025.journal").last_txn_date.as_deref(),
        Some("2025-12-19")
    );
    assert_eq!(find(&targets, "2026/2026.journal").label, "2026.journal");
}

#[test]
fn the_full_fledged_layout_ranks_by_content_not_by_the_root_being_called_all() {
    let targets = layout("full-fledged/all.journal");
    assert_eq!(
        ids(&targets),
        ["2018.journal", "2017.journal", "all.journal"]
    );
    let root = find(&targets, "all.journal");
    assert!(root.is_root, "a root is a root whatever it is called");
    assert_eq!(root.txn_count, 0, "it declares accounts and includes files");
}

#[test]
fn a_monthly_layout_ranks_by_date_where_alphabetical_order_disagrees() {
    let targets = layout("monthly/main.journal");
    // Alphabetically: february, january, march. By content: march is newest.
    assert_eq!(
        ids(&targets),
        [
            "march.journal",
            "february.journal",
            "january.journal",
            "main.journal",
        ]
    );
}

// ---------------------------------------------------------------------------
// The rule the module exists to keep
// ---------------------------------------------------------------------------

#[test]
fn directive_only_files_rank_below_every_file_that_holds_a_transaction() {
    // The assertion the plan calls for, stated over content and not over names:
    // in every layout, no file holding zero transactions may outrank one that
    // holds any.
    for root in [
        "single/main.journal",
        "split-year/main.journal",
        "full-fledged/all.journal",
        "monthly/main.journal",
    ] {
        let targets = layout(root);
        let first_empty = targets.iter().position(|t| t.txn_count == 0);
        let last_bearing = targets.iter().rposition(|t| t.txn_count > 0);
        if let (Some(empty), Some(bearing)) = (first_empty, last_bearing) {
            assert!(
                bearing < empty,
                "{root}: {:?} — a file with no transactions outranked one with some",
                ids(&targets)
            );
        }
    }
}

#[test]
fn a_directive_only_file_is_listed_and_flagged_rather_than_hidden() {
    // Someone's genuinely empty 2027.journal is a legitimate target on 1
    // January, so demotion must never become concealment.
    let targets = layout("split-year/main.journal");
    for id in ["accounts.journal", "prices.journal"] {
        let target = find(&targets, id);
        assert_eq!(target.txn_count, 0);
        assert_eq!(target.last_txn_date, None);
        assert!(target.writable);
    }
}

#[test]
fn a_price_file_is_not_promoted_by_the_dates_in_its_p_directives() {
    // prices.journal's newest line is 2026-06-30, later than every transaction
    // in the tree. Only transaction dates rank.
    let targets = layout("split-year/main.journal");
    assert_eq!(find(&targets, "prices.journal").last_txn_date, None);
    assert_eq!(
        ids(&targets).first(),
        Some(&"2026/2026.journal"),
        "the newest transaction, not the newest date of any kind"
    );
}

#[test]
fn the_root_is_always_listed_however_it_ranks() {
    for (root, id) in [
        ("single/main.journal", "main.journal"),
        ("split-year/main.journal", "main.journal"),
        ("full-fledged/all.journal", "all.journal"),
        ("monthly/main.journal", "main.journal"),
    ] {
        let targets = layout(root);
        assert!(find(&targets, id).is_root);
        assert_eq!(
            targets.iter().filter(|target| target.is_root).count(),
            1,
            "{root}: exactly one root"
        );
    }
}

#[test]
fn every_id_is_a_relative_forward_slash_path_and_never_a_location() {
    for root in [
        "single/main.journal",
        "split-year/main.journal",
        "full-fledged/all.journal",
        "monthly/main.journal",
    ] {
        for target in layout(root) {
            assert!(!target.id.starts_with('/'), "{}: absolute id", target.id);
            assert!(!target.id.contains('\\'), "{}: backslash in id", target.id);
            assert!(!target.id.contains(".."), "{}: traversal in id", target.id);
            assert!(
                target.label == target.id.rsplit('/').next().unwrap_or(&target.id),
                "{}: label is the file's own name",
                target.id
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Names that lie
// ---------------------------------------------------------------------------

/// A scratch journal tree that removes itself on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str, files: &[(&str, &str)]) -> Self {
        let dir =
            std::env::temp_dir().join(format!("ledgeline-journals-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        for (relative, text) in files {
            let path = dir.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("create scratch subdir");
            }
            std::fs::write(&path, text).expect("write scratch file");
        }
        Self(dir)
    }

    fn ranked(&self, root: &str) -> Vec<JournalTarget> {
        ranked(&self.0.join(root))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_file_called_prices_that_holds_the_newest_transactions_ranks_first() {
    // Every name in this tree lies. `prices.journal` holds the newest
    // transactions; `accounts.journal` holds older ones; `2026.journal` holds
    // nothing but declarations. A ranking that recognized any of those names
    // gets every position wrong.
    let scratch = Scratch::new(
        "liars",
        &[
            (
                "main.journal",
                "include 2026.journal\ninclude accounts.journal\ninclude prices.journal\n",
            ),
            (
                "2026.journal",
                "account a:b\naccount c:d\ncommodity $1,000.00\n",
            ),
            (
                "accounts.journal",
                "2026-01-04 older\n    a:b  $1.00\n    c:d\n",
            ),
            (
                "prices.journal",
                "2026-09-30 newest\n    a:b  $2.00\n    c:d\n",
            ),
        ],
    );
    let targets = scratch.ranked("main.journal");
    assert_eq!(
        ids(&targets),
        [
            "prices.journal",
            "accounts.journal",
            "main.journal",
            "2026.journal",
        ],
        "the ranking must follow the transactions, not the names"
    );
}

#[test]
fn an_empty_new_year_file_is_offered_even_though_it_ranks_last() {
    // The 1-January case from the plan: a file created for the coming year holds
    // nothing yet and is exactly the file the user means to import into.
    let scratch = Scratch::new(
        "new-year",
        &[
            (
                "main.journal",
                "include 2026.journal\ninclude 2027.journal\n",
            ),
            (
                "2026.journal",
                "2026-12-30 last of the year\n    a:b  $1.00\n    c:d\n",
            ),
            ("2027.journal", ""),
        ],
    );
    let targets = scratch.ranked("main.journal");
    assert_eq!(
        ids(&targets),
        ["2026.journal", "main.journal", "2027.journal"]
    );
    let empty = find(&targets, "2027.journal");
    assert_eq!(empty.txn_count, 0);
    assert!(empty.writable, "it exists and is a regular file");
}

#[test]
fn the_root_is_whichever_file_was_opened_not_whichever_is_named_like_one() {
    // The same tree, opened two ways. `is_root` follows the file Ledgeline was
    // pointed at; opening the year file directly makes it the root and drops the
    // parent it is normally included from, because the parent is no longer part
    // of this journal at all.
    let scratch = Scratch::new(
        "reroot",
        &[
            ("main.journal", "include 2026.journal\n"),
            ("2026.journal", "2026-01-01 a\n    a:b  $1.00\n    c:d\n"),
        ],
    );
    assert_eq!(
        ids(&scratch.ranked("main.journal")),
        ["2026.journal", "main.journal"]
    );

    let rerooted = scratch.ranked("2026.journal");
    assert_eq!(ids(&rerooted), ["2026.journal"]);
    assert!(find(&rerooted, "2026.journal").is_root);
}
