//! Which journal file an import should be written to (WP-11).
//!
//! `hledger import` appends to a file the user names. Naming it well is the
//! whole of this module: given the journal Ledgeline already has open, rank
//! every file that fed it so the right one can be pre-selected.
//!
//! # No filename is ever inspected
//!
//! This is the rule the module exists to keep. Real layouts in the wild include
//! a single file with `account` declarations at the top and transactions below;
//! `main.journal` including `accounts.journal`, `prices.journal`,
//! `2025/2025.journal` and `2026/2026.journal`; the full-fledged-hledger
//! convention of `all.journal` including `2017.journal` and `2018.journal`; and
//! one file per month. Any rule spelled in filenames fails on at least two of
//! those, and a rule that recognized `prices.journal` would still cheerfully
//! offer `prices.journal` to someone whose price file is called `rates.hledger`.
//!
//! So the ranking is derived from **content only**:
//!
//! 1. Files holding transactions rank above files holding none, ordered by
//!    [`JournalTarget::last_txn_date`] **descending** — the file whose newest
//!    transaction is closest to today first. That one rule gives the right answer
//!    for year files, month files, a single file and per-account files alike.
//! 2. Files holding no transactions — a pure `account`/`commodity`/`P` directive
//!    file — rank **last** and are flagged by their zero `txn_count`. They are
//!    never hidden: someone's genuinely empty `2027.journal` is a legitimate
//!    target on 1 January, and so is a brand-new file that only declares
//!    accounts.
//! 3. Ties keep the order the parse read the files in, which is the main file
//!    followed by each `include` in first-read order. Deterministic, and derived
//!    from the journal's own structure rather than from any name.
//!
//! The root journal is always listed regardless of where it ranks, flagged with
//! [`JournalTarget::is_root`].
//!
//! `label` exists for display and is the file's own name; nothing in the ranking
//! reads it. A test that passes because a name was recognized is a failing test
//! for this module.
//!
//! # A projection, not a scan
//!
//! [`Journal::source_files`] already lists every file the parse read — including
//! the `include`s that contribute only directives — and every
//! [`crate::model::Transaction`] already records the file it came from. So this
//! is a fold over data already in hand. The one storage call is the `stat`
//! behind [`JournalTarget::writable`] ([`Vault::stat`], which never follows a
//! link), and it asks about the file's *type*, never its contents.

use crate::model::Journal;
use crate::vault::{LocalVault, Vault, VaultPath};
use std::collections::HashMap;
use std::path::Path;

/// One journal file an import could be written to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalTarget {
    /// The path relative to the include root (the main journal file's own
    /// directory, which is what `include` is confined to), forward-slash
    /// separated. This is the handle a caller addresses the file by.
    ///
    /// Never an absolute path: like the rules API's ids, nothing here echoes a
    /// location back to a client. A file that somehow sits outside the include
    /// root falls back to its bare name for the same reason.
    pub id: String,
    /// Display name: the last component of [`JournalTarget::id`]. Display only —
    /// the id disambiguates two files of the same name in different directories,
    /// and **no ranking decision reads this**.
    pub label: String,
    /// How many transactions this file holds. Zero means a pure directive file
    /// (or an empty one), which is what demotes it — see the module docs.
    pub txn_count: usize,
    /// The newest transaction date in this file, `YYYY-MM-DD`, or `None` when it
    /// holds none. The parser normalizes every date to ISO, so the lexical
    /// maximum is the chronological one.
    pub last_txn_date: Option<String>,
    /// This is the journal Ledgeline was opened with, as opposed to something it
    /// `include`s.
    pub is_root: bool,
    /// A regular file, not a symlink, inside the include root.
    ///
    /// Deliberately a claim about the file's *type and location*, not about OS
    /// permissions: a mode bit says nothing useful about whether the write will
    /// land (ownership, ACLs and read-only mounts all outrank it), and the write
    /// path reports the real answer. What this rules out is the shapes that are
    /// wrong to write to *whatever* the permissions say — a symlink pointing out
    /// of the tree, a directory, a FIFO.
    pub writable: bool,
}

/// Rank every file `journal` was parsed from, best-first, judging
/// [`JournalTarget::writable`] on disk beside the main file.
///
/// See the module docs for the ranking and for why it never looks at a name.
#[must_use]
pub fn targets(journal: &Journal) -> Vec<JournalTarget> {
    let root = journal
        .source_files
        .first()
        .and_then(|main| main.parent())
        .unwrap_or_else(|| Path::new("."));
    targets_in(&LocalVault::new(root), journal)
}

/// [`targets`] for a journal parsed through `vault` ([`crate::parse::parse_journal_in`]),
/// whose source-file keys are [`Vault::key`]s.
#[must_use]
pub fn targets_in(vault: &dyn Vault, journal: &Journal) -> Vec<JournalTarget> {
    let include_root = journal
        .source_files
        .first()
        .and_then(|main| vault.vault_path(main).ok())
        .and_then(|main| main.parent())
        .unwrap_or_default();
    let tallies = tally(journal);

    let mut targets: Vec<JournalTarget> = journal
        .source_files
        .iter()
        .enumerate()
        .map(|(at, key)| {
            let (txn_count, last_txn_date) = tallies
                .get(key.as_path())
                .map_or((0, None), |&(count, last)| {
                    (count, last.map(str::to_string))
                });
            let path = vault
                .vault_path(key)
                .ok()
                .filter(|path| path.starts_with(&include_root) && *path != include_root);
            let id = identify(&include_root, path.as_ref(), key);
            JournalTarget {
                label: id.rsplit('/').next().unwrap_or(&id).to_string(),
                id,
                txn_count,
                last_txn_date,
                is_root: at == 0,
                writable: path.is_some_and(|path| vault.is_file(&path)),
            }
        })
        .collect();

    // A STABLE sort, so files that tie fall back to the order the parse read
    // them in — the main file, then each `include`. `false < true`, so
    // transaction-bearing files sort ahead of directive-only ones.
    targets.sort_by(|a, b| {
        (a.txn_count == 0)
            .cmp(&(b.txn_count == 0))
            .then_with(|| b.last_txn_date.cmp(&a.last_txn_date))
    });
    targets
}

/// Per source file: how many transactions it holds, and the newest date among
/// them.
///
/// Keyed by [`crate::model::Transaction::source_file`], which is the same
/// resolved absolute path [`Journal::source_files`] records, so the two always
/// agree about which file is which.
fn tally(journal: &Journal) -> HashMap<&Path, (usize, Option<&str>)> {
    journal
        .transactions
        .iter()
        .fold(HashMap::new(), |mut tallies, txn| {
            let entry = tallies
                .entry(txn.source_file.as_path())
                .or_insert((0, None));
            entry.0 += 1;
            if entry.1.is_none_or(|newest| txn.date.as_str() > newest) {
                entry.1 = Some(txn.date.as_str());
            }
            tallies
        })
}

/// A file's id: its vault path relative to `root` when it sits below it, else
/// its key's bare name — never an absolute path.
fn identify(root: &VaultPath, path: Option<&VaultPath>, key: &Path) -> String {
    path.and_then(|path| relative_id(root, path))
        .or_else(|| {
            key.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_default()
}

/// `path` relative to `root`, forward-slash separated, or `None` if it is not
/// strictly below `root`.
///
/// A [`VaultPath`] holds only plain components, so an id cannot contain a `.`,
/// a `..`, a root or a Windows prefix — the same guarantee the rules-file ids
/// hold to — and therefore cannot mean anything other than "this file, below
/// the root". `join` rather than `Path::display`: an id is a wire string with
/// one separator on every platform, not a local path.
fn relative_id(root: &VaultPath, path: &VaultPath) -> Option<String> {
    (path != root && path.starts_with(root)).then(|| {
        path.segments()
            .skip(root.segments().count())
            .collect::<Vec<_>>()
            .join("/")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::MemVault;

    fn vp(s: &str) -> VaultPath {
        VaultPath::new(s).unwrap()
    }

    #[test]
    fn an_id_is_a_forward_slash_relative_path_of_plain_components() {
        let root = vp("books");
        assert_eq!(
            relative_id(&root, &vp("books/2026/2026.journal")).as_deref(),
            Some("2026/2026.journal")
        );
        assert_eq!(
            relative_id(&root, &vp("books/main.journal")).as_deref(),
            Some("main.journal")
        );
        // Not below the root, and the root itself, both decline.
        assert_eq!(relative_id(&root, &vp("elsewhere/x.journal")), None);
        assert_eq!(relative_id(&root, &root), None);
    }

    #[test]
    fn a_file_outside_the_include_root_falls_back_to_its_bare_name() {
        // Never an absolute path: an id is a handle, not a location.
        assert_eq!(
            identify(&vp("books"), None, Path::new("/elsewhere/other.journal")),
            "other.journal"
        );
    }

    #[test]
    fn writable_means_a_regular_file_below_the_root_in_the_vault() {
        let vault = MemVault::new()
            .with_file(
                "main.journal",
                "include 2026.journal\ninclude linked.journal\n",
            )
            .with_file("2026.journal", "2026-01-01 x\n  a  1\n  b\n");
        vault.insert_symlink("linked.journal", "2026.journal");
        let parsed = crate::parse::parse_journal_in(
            &vault,
            &vp("main.journal"),
            &std::collections::HashMap::new(),
        )
        .unwrap();
        let mut journal = parsed.journal;
        // A source the parse could not have produced: outside the vault.
        journal
            .source_files
            .push(std::path::PathBuf::from("/etc/hosts"));
        let targets = targets_in(&vault, &journal);
        let by_id = |id: &str| targets.iter().find(|t| t.id == id).unwrap().clone();
        assert!(by_id("main.journal").writable);
        assert!(by_id("main.journal").is_root);
        assert!(by_id("2026.journal").writable);
        // Included twice — once through the in-vault link, which resolves to the
        // same file — so it holds two parsed transactions, one source entry.
        assert_eq!(by_id("2026.journal").txn_count, 2);
        assert!(targets.iter().all(|t| t.id != "linked.journal"));
        assert!(
            !by_id("hosts").writable,
            "nothing outside the root is writable"
        );
    }
}
