//! The directory walk [`crate::rules::discovery`] and
//! [`crate::projections::discovery`] share.
//!
//! Both surfaces answer the same question about the same tree — *which files
//! below the journal's own directory may this feature read, and in one case
//! write* — and until now each had its own copy of the answer. The copies were
//! line for line the same, down to the cap that stops one enormous directory
//! defeating the entry budget by allocating, the reversed sub-directory stack,
//! and the order the checks run in; each carried a comment asking the next
//! reader to keep it in step with the other by hand.
//!
//! That is not a tidiness complaint. The symlinked-parent gap on the *create*
//! path existed in `rules`, was noticed only because somebody sat down and
//! wrote the second copy, and then had to be carried back across by hand. A
//! guard whose only defect-detector is "write it again" costs exactly as much
//! to check as it costs to share, so it is shared here, once.
//!
//! # Through the vault
//!
//! The walk lists directories with [`Vault::list_dir`], which describes each
//! entry itself and never a symlink's target — so a link is seen as a link,
//! here as before, and refused. Every path is a [`VaultPath`], which cannot
//! name anything outside the vault.
//!
//! # What the two callers still own
//!
//! Which names they list, how many, how deep, what a symlink warning says, and
//! what they build out of an admitted entry — [`ScanSpec`] and the `describe`
//! hook. Nothing else: a knob is a way for two scans to drift, so the entry and
//! warning budgets, the order of the checks and every warning string but one
//! are fixed here rather than passed in.
//!
//! Each caller also keeps its own path newtype (`RulesPath`,
//! `ProjectionPath`). This module hands back a plain [`Entry`], because "you
//! may only write to a file discovery returned" is a property of *those* types
//! — they have no public constructor, and there is deliberately one per feature
//! so a scan of one surface can never mint a write token for the other.
//!
//! # What lives in `parse.rs` instead
//!
//! The containment test ([`crate::parse::confine`]) stays beside the `include`
//! guard it was extracted from — as does [`crate::parse::resolve_new_in`], the
//! create-path guard chain. One file holds every hand-rolled traversal check in
//! the crate; that is the whole reason `confine` is not a method here.

use crate::vault::{Entry, EntryKind, FileIdentity, Vault, VaultPath};

// ---------------------------------------------------------------------------
// Budgets that are not a caller's business
// ---------------------------------------------------------------------------

/// How many directory entries are EXAMINED — **the load-bearing bound**.
///
/// [`ScanSpec::max_files`] and [`ScanSpec::max_depth`] bound the *answer*; this
/// one bounds the *work*. A user whose journal lives in `$HOME` (a completely
/// ordinary choice) would otherwise turn one scan into a full-disk walk, and no
/// skip list can prevent that in general: [`ScanSpec::skip_dirs`] names
/// directories that are commonly enormous, not the ones that happen to be
/// enormous here. Only counting entries and stopping does. When it trips,
/// [`Scanned::truncated`] is set, so a user is told the list is a subset rather
/// than shown a subset that looks complete.
pub(crate) const MAX_SCAN_ENTRIES: usize = 20_000;

/// How many scan-level warnings are kept, before one path-free note says there
/// were more.
///
/// A hostile or merely unlucky tree can produce a warning per skipped entry, up
/// to [`MAX_SCAN_ENTRIES`] of them, and these strings reach a dialog. Bounding
/// the answer is the same discipline as bounding the walk.
pub(crate) const MAX_SCAN_WARNINGS: usize = 100;

// ---------------------------------------------------------------------------
// What differs between the two scans
// ---------------------------------------------------------------------------

/// What one scan does differently from the other.
pub(crate) struct ScanSpec<'a> {
    /// Whether a file NAME is one this scan lists. Applied to the entry's own
    /// name and never to a path: every name it sees came out of a directory
    /// listing moments earlier.
    pub(crate) name_ok: fn(&str) -> bool,
    /// Directory names never descended into.
    ///
    /// A field rather than a constant on purpose. This is a *performance*
    /// courtesy and not a security control — [`MAX_SCAN_ENTRIES`] is the
    /// control — so the two scans are free to disagree about what is worth
    /// skipping, and a single shared list would make one of them wrong quietly.
    /// Every entry whose name starts with `.` is skipped regardless, which is
    /// what keeps `.git/` out.
    pub(crate) skip_dirs: &'a [&'a str],
    /// How far below the root the walk descends. The root itself is depth 0.
    pub(crate) max_depth: usize,
    /// How many files are RETURNED before the listing is truncated.
    pub(crate) max_files: usize,
    /// The clause after the semicolon in the symlink warning.
    ///
    /// The one string the two scans do not share, because one of them only
    /// reads the files it finds and the other also writes to them, and a
    /// warning that got that wrong would be telling the user something untrue
    /// about what Ledgeline is about to do.
    pub(crate) symlink_note: &'a str,
}

/// What one walk produced, before the caller sorts it and names it.
pub(crate) struct Scanned<T> {
    /// The scan root, handed straight back so the caller can keep it private —
    /// no absolute path leaves either discovery module.
    pub(crate) root: VaultPath,
    /// One per admitted entry, in walk order. **Unsorted**: the two callers
    /// group their listings differently, and sorting here as well would be the
    /// slower way to reach the same answer.
    pub(crate) files: Vec<T>,
    /// Each naming a **relative** path only, bounded by [`MAX_SCAN_WARNINGS`].
    /// Plain `String`s because one caller wraps them in a richer warning type
    /// and the other does not.
    pub(crate) warnings: Vec<String>,
    /// A cap was hit and the list is incomplete.
    pub(crate) truncated: bool,
}

/// Walk `root` in `vault`, admitting every entry `spec` allows and calling
/// `describe` on each one.
///
/// `describe` is handed the entry's relative id, its file name and the [`Entry`]
/// the listing produced — so a caller's `size_bytes` and recorded identity
/// always describe the same listing the walk decided on.
///
/// Infallible, like the parser: an unreadable directory, a symlink and a
/// non-regular file each become a warning naming a relative path. Showing the
/// user what is there is the whole point of the screen, and one unreadable
/// directory is not a reason to show nothing.
pub(crate) fn scan<T, D>(
    vault: &dyn Vault,
    root: VaultPath,
    spec: &ScanSpec<'_>,
    describe: D,
) -> Scanned<T>
where
    D: FnMut(String, &str, &Entry) -> T,
{
    Scan {
        vault,
        root,
        spec,
        describe,
        files: Vec::new(),
        warnings: Vec::new(),
        truncated: false,
        examined: 0,
    }
    .run()
}

// ---------------------------------------------------------------------------
// The walk
// ---------------------------------------------------------------------------

/// One scan in progress.
///
/// Iterative with an explicit stack, never recursive: the depth cap already
/// bounds a well-behaved tree, but recursion turns a *mistake* in that bound
/// into a stack overflow, which is a `SIGABRT` and not a catchable panic. That
/// is the failure mode SEC-4 fixed on the include path, and it is not worth
/// reintroducing for the sake of four fewer lines.
struct Scan<'a, T, D> {
    vault: &'a dyn Vault,
    root: VaultPath,
    spec: &'a ScanSpec<'a>,
    describe: D,
    files: Vec<T>,
    warnings: Vec<String>,
    truncated: bool,
    examined: usize,
}

impl<T, D> Scan<'_, T, D>
where
    D: FnMut(String, &str, &Entry) -> T,
{
    /// Walk the tree, depth-first, visiting each directory's entries in name
    /// order.
    ///
    /// The order matters for more than tidiness: when a cap trips, *which*
    /// entries were examined decides which files survive, and listing order is
    /// filesystem- and even run-dependent. Sorting each directory makes a
    /// truncated result reproducible instead of arbitrary.
    fn run(mut self) -> Scanned<T> {
        let mut stack = vec![(self.root.clone(), 0usize)];
        while let Some((dir, depth)) = stack.pop() {
            let Some(children) = self.read_dir(&dir) else {
                continue;
            };
            let mut subdirs = Vec::new();
            for entry in children {
                // Both budgets are checked in exactly one place, before any
                // work is done for the entry.
                if self.examined >= MAX_SCAN_ENTRIES || self.files.len() >= self.spec.max_files {
                    self.truncated = true;
                    return self.finish();
                }
                self.examined += 1;
                self.visit(entry, depth, &mut subdirs);
            }
            // Reversed, so the stack pops them back in name order.
            stack.extend(subdirs.into_iter().rev().map(|dir| (dir, depth + 1)));
        }
        self.finish()
    }

    fn finish(self) -> Scanned<T> {
        Scanned {
            root: self.root,
            files: self.files,
            warnings: self.warnings,
            truncated: self.truncated,
        }
    }

    /// This directory's entries, sorted by path, and never more than the
    /// remaining entry budget.
    ///
    /// The cap is not decoration: listing a directory of a million names before
    /// checking the budget would let a single directory defeat
    /// [`MAX_SCAN_ENTRIES`] by allocating instead of by walking. One entry past
    /// the budget is taken so the caller's check still trips and sets
    /// `truncated`.
    fn read_dir(&mut self, dir: &VaultPath) -> Option<Vec<Entry>> {
        let cap = MAX_SCAN_ENTRIES.saturating_sub(self.examined) + 1;
        match self.vault.list_dir(dir, Some(cap)) {
            Ok(mut children) => {
                children.sort_by(|a, b| a.path.cmp(&b.path));
                Some(children)
            }
            Err(_) => {
                // The vault error is deliberately not quoted. It is very
                // probably path-free, but "very probably" is not a property
                // this module can assert about every backend's error text, and
                // this string reaches a dialog.
                let message = match relative_id(&self.root, dir) {
                    Some(id) => {
                        format!(
                            "the directory {id} could not be read; anything inside it was skipped"
                        )
                    }
                    None => "the journal's own directory could not be read".to_string(),
                };
                self.warn(message);
                None
            }
        }
    }

    /// Classify one directory entry. The order of the checks is the order of the
    /// guarantees, and it is **not** the order they could be run in most
    /// cheaply: a symlink is refused before a hidden name is skipped, so a
    /// `.hidden` link still says so.
    fn visit(&mut self, entry: Entry, depth: usize, subdirs: &mut Vec<VaultPath>) {
        let Some(id) = relative_id(&self.root, &entry.path) else {
            // Not below the root: a listing cannot produce one, so there is
            // nothing true to say about it.
            return;
        };
        let name = entry.name().to_string();

        // The listing describes the entry itself, never what a link points at:
        // the whole point is to see the link.
        if entry.kind == EntryKind::Symlink {
            self.warn(format!(
                "{id} is a symbolic link and was skipped; {}",
                self.spec.symlink_note
            ));
            return;
        }

        // A leading dot, on a directory OR on a file. A hidden entry is one the
        // user's own file browser does not show them, so offering it is offering
        // something they cannot see — and a dot-file in a journal directory is
        // much more often a tool's leftover than a file someone wants listed.
        // For directories this is what keeps `.git/` and `.direnv/` out. Silent,
        // like `skip_dirs`: a policy skip is not a problem to report.
        if name.starts_with('.') {
            return;
        }

        if entry.kind == EntryKind::Dir {
            if self.spec.skip_dirs.contains(&name.as_str()) {
                return;
            }
            if depth + 1 > self.spec.max_depth {
                self.truncated = true;
                return;
            }
            subdirs.push(entry.path);
            return;
        }

        let named = (self.spec.name_ok)(&name);
        if entry.kind != EntryKind::File {
            // FIFOs, devices, sockets. A FIFO is the one that actually bites: a
            // `read` on one with no writer blocks forever, which would hang the
            // request that asked for the list, not merely fail it.
            if named {
                self.warn(format!("{id} is not a regular file and was skipped"));
            }
            return;
        }
        if !named {
            return;
        }

        let file = (self.describe)(id, &name, &entry);
        self.files.push(file);
    }

    /// Record a scan-level warning, up to [`MAX_SCAN_WARNINGS`] of them plus one
    /// path-free note saying there were more.
    fn warn(&mut self, message: String) {
        if self.warnings.len() < MAX_SCAN_WARNINGS {
            self.warnings.push(message);
        } else if self.warnings.len() == MAX_SCAN_WARNINGS {
            self.warnings.push(format!(
                "more than {MAX_SCAN_WARNINGS} entries were skipped; only the first \
                 {MAX_SCAN_WARNINGS} are listed"
            ));
        }
    }
}

// ---------------------------------------------------------------------------
// Ids and identity
// ---------------------------------------------------------------------------

/// `path` relative to `root`, forward-slash separated, or `None` if it is not
/// strictly below `root`.
///
/// A [`VaultPath`] already holds no `.`, `..`, root or prefix component, so a
/// well-formed id cannot mean anything other than "this file, below the root".
pub(crate) fn relative_id(root: &VaultPath, path: &VaultPath) -> Option<String> {
    if path == root || !path.starts_with(root) {
        return None;
    }
    let depth = root.segments().count();
    Some(path.segments().skip(depth).collect::<Vec<_>>().join("/"))
}

/// Re-`stat` `path` (never following a link) and require it is still a regular
/// file with the identity recorded at scan time.
///
/// **Called immediately before a write**, and one half of a pair — the caller
/// re-reads and re-fingerprints the bytes too, because a freed inode number can
/// be handed straight back to the next create. `identity` of `None` degrades
/// this to the regular-file check alone, which is weaker, and honestly so: it
/// can still refuse a name that became a link or a device, but not one that
/// became a different regular file.
pub(crate) fn still_the_same_file(
    vault: &dyn Vault,
    path: &VaultPath,
    identity: Option<FileIdentity>,
) -> bool {
    matches!(
        vault.stat(path),
        Ok(Some(entry)) if entry.is_file() && entry.identity == identity
    )
}

// ---------------------------------------------------------------------------
// Tests — the pure helper, and the walk over an in-memory vault. The guards
// are also exercised against a real filesystem in `tests/rules_security.rs` and
// `tests/projection_files.rs`.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::MemVault;

    fn vp(s: &str) -> VaultPath {
        VaultPath::new(s).unwrap()
    }

    #[test]
    fn ids_are_relative_forward_slash_paths_of_plain_components() {
        let root = VaultPath::root();
        assert_eq!(
            relative_id(&root, &vp("plans/p.journal")).as_deref(),
            Some("plans/p.journal")
        );
        assert_eq!(
            relative_id(&vp("books"), &vp("books/import/2026/b.rules")).as_deref(),
            Some("import/2026/b.rules")
        );
        assert_eq!(relative_id(&root, &root), None, "the root itself");
        assert_eq!(relative_id(&vp("books"), &vp("other/b.rules")), None);
        assert_eq!(relative_id(&vp("books"), &vp("booksy/b.rules")), None);
    }

    fn spec() -> ScanSpec<'static> {
        ScanSpec {
            name_ok: |name| name.ends_with(".rules"),
            skip_dirs: &["node_modules"],
            max_depth: 2,
            max_files: 10,
            symlink_note: "only real files are read",
        }
    }

    #[test]
    fn the_walk_refuses_links_skips_hidden_and_skipped_dirs_and_caps_depth() {
        let vault = MemVault::new()
            .with_file("books/a.rules", "")
            .with_file("books/import/b.rules", "")
            .with_file("books/import/deep/deeper/c.rules", "")
            .with_file("books/.git/hidden.rules", "")
            .with_file("books/node_modules/x.rules", "")
            .with_file("books/notes.txt", "")
            .with_file("outside.rules", "");
        vault.insert_symlink("books/linked.rules", "a.rules");
        let scanned = scan(&vault, vp("books"), &spec(), |id, name, entry| {
            (id, name.to_string(), entry.path.clone())
        });
        let mut ids: Vec<String> = scanned.files.iter().map(|(id, ..)| id.clone()).collect();
        ids.sort();
        assert_eq!(ids, vec!["a.rules", "import/b.rules"]);
        assert!(scanned.truncated, "the depth cap tripped");
        assert_eq!(
            scanned.warnings,
            vec!["linked.rules is a symbolic link and was skipped; only real files are read"]
        );
        assert_eq!(scanned.root, vp("books"));
    }

    #[test]
    fn an_unreadable_root_is_one_path_free_warning() {
        let vault = MemVault::new();
        let scanned = scan(&vault, vp("missing"), &spec(), |id, _, _| id);
        assert!(scanned.files.is_empty());
        assert_eq!(
            scanned.warnings,
            vec!["the journal's own directory could not be read"]
        );
    }

    #[test]
    fn the_same_file_check_compares_identity_and_refuses_links() {
        let vault = MemVault::new().with_file("a.rules", "x");
        let entry = vault.stat(&vp("a.rules")).unwrap().unwrap();
        assert!(still_the_same_file(&vault, &vp("a.rules"), entry.identity));
        vault.modify_externally("a.rules", "x");
        assert!(!still_the_same_file(&vault, &vp("a.rules"), entry.identity));
        vault.insert_symlink("a.rules", "b.rules");
        assert!(!still_the_same_file(&vault, &vp("a.rules"), None));
    }
}
