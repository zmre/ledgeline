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
//! is a fold over data already in hand. The one filesystem call is the `stat`
//! behind [`JournalTarget::writable`], which asks about the file's *type*, never
//! its contents.

use crate::model::Journal;
use std::collections::HashMap;
use std::path::{Component, Path};

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

/// Rank every file `journal` was parsed from, best-first.
///
/// See the module docs for the ranking and for why it never looks at a name.
#[must_use]
pub fn targets(journal: &Journal) -> Vec<JournalTarget> {
    let include_root = journal
        .source_files
        .first()
        .and_then(|main| main.parent())
        .unwrap_or_else(|| Path::new("."));
    let tallies = tally(journal);

    let mut targets: Vec<JournalTarget> = journal
        .source_files
        .iter()
        .enumerate()
        .map(|(at, path)| {
            let (txn_count, last_txn_date) = tallies
                .get(path.as_path())
                .map_or((0, None), |&(count, last)| {
                    (count, last.map(str::to_string))
                });
            let id = identify(include_root, path);
            JournalTarget {
                label: id.rsplit('/').next().unwrap_or(&id).to_string(),
                id,
                txn_count,
                last_txn_date,
                is_root: at == 0,
                writable: writable(include_root, path),
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

/// `path`'s id: relative to `root` when it sits below it, else its bare name.
fn identify(root: &Path, path: &Path) -> String {
    relative_id(root, path)
        .or_else(|| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_default()
}

/// `path` relative to `root`, forward-slash separated, or `None` if it is not
/// below `root` or has any component that is not a plain UTF-8 name.
///
/// Requiring every component to be [`Component::Normal`] is a guard rather than
/// tidiness — the same one the rules-file ids hold to. It is what makes it
/// impossible for a `.`, a `..`, a root or a Windows prefix to appear inside an
/// id, and therefore impossible for a well-formed id to mean anything other than
/// "this file, below the root".
fn relative_id(root: &Path, path: &Path) -> Option<String> {
    let parts: Option<Vec<&str>> = path
        .strip_prefix(root)
        .ok()?
        .components()
        .map(|component| match component {
            Component::Normal(name) => name.to_str(),
            _ => None,
        })
        .collect();
    parts.filter(|parts| !parts.is_empty()).map(|parts| {
        // `join` rather than `Path::display`: an id is a wire string with one
        // separator on every platform, not a local path.
        parts.join("/")
    })
}

/// Is `path` a regular file, not a symlink, inside `root`?
///
/// [`std::fs::symlink_metadata`] does not follow links, so a symlink answers
/// `false` on its own file type without a second question being asked.
fn writable(root: &Path, path: &Path) -> bool {
    path.starts_with(root)
        && std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_id_is_a_forward_slash_relative_path_of_plain_components() {
        let root = Path::new("/books");
        assert_eq!(
            relative_id(root, Path::new("/books/2026/2026.journal")).as_deref(),
            Some("2026/2026.journal")
        );
        assert_eq!(
            relative_id(root, Path::new("/books/main.journal")).as_deref(),
            Some("main.journal")
        );
        // Not below the root, and the root itself, both decline.
        assert_eq!(relative_id(root, Path::new("/elsewhere/x.journal")), None);
        assert_eq!(relative_id(root, root), None);
    }

    #[test]
    fn a_file_outside_the_include_root_falls_back_to_its_bare_name() {
        // Never an absolute path: an id is a handle, not a location.
        assert_eq!(
            identify(Path::new("/books"), Path::new("/elsewhere/other.journal")),
            "other.journal"
        );
    }

    #[test]
    fn nothing_outside_the_include_root_is_writable() {
        assert!(!writable(Path::new("/books"), Path::new("/etc/hosts")));
    }
}
