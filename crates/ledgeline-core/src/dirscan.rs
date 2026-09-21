//! The directory walk [`crate::rules::discovery`] and
//! [`crate::projections::discovery`] share.
//!
//! Both surfaces answer the same question about the same tree — *which files
//! below the journal's own directory may this feature read, and in one case
//! write* — and until now each had its own copy of the answer. The copies were
//! line for line the same, down to the `take` that stops one enormous directory
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
//! # What the two callers still own
//!
//! Which names they list, how many, how deep, what a symlink warning says, and
//! what they build out of an admitted entry — [`ScanSpec`] and the `describe`
//! hook. Nothing else: a knob is a way for two scans to drift, so the entry and
//! warning budgets, the order of the checks and every warning string but one
//! are fixed here rather than passed in.
//!
//! Each caller also keeps its own path newtype (`RulesPath`,
//! `ProjectionPath`). This module hands back a plain [`PathBuf`], because "you
//! may only write to a file discovery returned" is a property of *those* types
//! — they have no public constructor, and there is deliberately one per feature
//! so a scan of one surface can never mint a write token for the other.
//!
//! # What lives in `parse.rs` instead
//!
//! The scan root ([`crate::parse::include_root_for`]) and the containment test
//! ([`crate::parse::confine`]) were already shared, and stay beside the
//! `include` guard they were extracted from — as does
//! [`crate::parse::resolve_new_in`], the create-path guard chain. One file
//! holds every hand-rolled traversal check in the crate; that is the whole
//! reason `confine` is not a method here.

use crate::parse;
use std::fs::{DirEntry, Metadata};
use std::path::{Component, Path, PathBuf};

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
    /// name and never to a path: every name it sees came out of `read_dir`
    /// moments earlier.
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
    /// The canonical scan root, handed straight back so the caller can keep it
    /// private — no absolute path leaves either discovery module.
    pub(crate) root: PathBuf,
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

/// Walk `root`, admitting every entry `spec` allows and calling `describe` on
/// each one.
///
/// `describe` is handed the entry's relative id, its file name, its **confined**
/// canonical path and the `symlink_metadata` that admitted it — so a caller's
/// `size_bytes` and recorded `(dev, ino)` always describe the same `stat` the
/// walk decided on.
///
/// Infallible, like the parser: an unreadable directory, a symlink and a
/// non-regular file each become a warning naming a relative path. Showing the
/// user what is there is the whole point of the screen, and one unreadable
/// directory is not a reason to show nothing.
pub(crate) fn scan<T, D>(root: PathBuf, spec: &ScanSpec<'_>, describe: D) -> Scanned<T>
where
    D: FnMut(String, &str, PathBuf, &Metadata) -> T,
{
    Scan {
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
    root: PathBuf,
    spec: &'a ScanSpec<'a>,
    describe: D,
    files: Vec<T>,
    warnings: Vec<String>,
    truncated: bool,
    examined: usize,
}

impl<T, D> Scan<'_, T, D>
where
    D: FnMut(String, &str, PathBuf, &Metadata) -> T,
{
    /// Walk the tree, depth-first, visiting each directory's entries in name
    /// order.
    ///
    /// The order matters for more than tidiness: when a cap trips, *which*
    /// entries were examined decides which files survive, and `read_dir` order
    /// is filesystem- and even run-dependent. Sorting each directory makes a
    /// truncated result reproducible instead of arbitrary.
    fn run(mut self) -> Scanned<T> {
        let mut stack = vec![(self.root.clone(), 0usize)];
        while let Some((dir, depth)) = stack.pop() {
            let Some(children) = self.read_dir(&dir) else {
                continue;
            };
            let mut subdirs = Vec::new();
            for (path, entry) in children {
                // Both budgets are checked in exactly one place, before any
                // work is done for the entry.
                if self.examined >= MAX_SCAN_ENTRIES || self.files.len() >= self.spec.max_files {
                    self.truncated = true;
                    return self.finish();
                }
                self.examined += 1;
                self.visit(&path, &entry, depth, &mut subdirs);
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
    /// The `take` is not decoration: collecting a directory of a million names
    /// before checking the budget would let a single directory defeat
    /// [`MAX_SCAN_ENTRIES`] by allocating instead of by walking. One entry past
    /// the budget is taken so the caller's check still trips and sets
    /// `truncated`.
    ///
    /// The [`DirEntry`] is kept beside its path, not dropped for it: its
    /// `file_type` is the `d_type` `read_dir` already returned, which is what
    /// lets [`Self::visit`] classify an entry without a `stat`.
    ///
    /// A per-entry `read_dir` error (a name that vanished mid-walk) drops that
    /// entry silently; there is nothing to say about it that is both true and
    /// useful.
    fn read_dir(&mut self, dir: &Path) -> Option<Vec<(PathBuf, DirEntry)>> {
        match std::fs::read_dir(dir) {
            Ok(entries) => {
                let mut children: Vec<(PathBuf, DirEntry)> = entries
                    .filter_map(Result::ok)
                    .map(|entry| (entry.path(), entry))
                    .take(MAX_SCAN_ENTRIES.saturating_sub(self.examined) + 1)
                    .collect();
                children.sort_by(|(a, _), (b, _)| a.cmp(b));
                Some(children)
            }
            Err(_) => {
                // The `io::Error` is deliberately not quoted. It is very
                // probably path-free, but "very probably" is not a property this
                // module can assert about every platform's error text, and this
                // string reaches a dialog.
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
    fn visit(&mut self, path: &Path, entry: &DirEntry, depth: usize, subdirs: &mut Vec<PathBuf>) {
        let (Some(id), Some(name)) = (
            relative_id(&self.root, path),
            path.file_name().and_then(std::ffi::OsStr::to_str),
        ) else {
            // A name that is not valid UTF-8, or a component that is not a plain
            // name. Skipped rather than lossily converted: two different names
            // can lossily convert to the SAME id, and an id that resolves to the
            // wrong file is a write to the wrong file.
            self.warn(format!(
                "{} has a name that is not valid UTF-8 and was skipped",
                lossy_relative(&self.root, path)
            ));
            return;
        };

        // `DirEntry::file_type`, never `metadata`: the whole point is to see the
        // link rather than what it points at — and on Linux and macOS this is
        // the `d_type` the directory read already returned, so classifying an
        // entry costs no syscall at all. Only an entry that survives every check
        // below goes on to pay for a `stat`, which is what keeps a tree full of
        // `.git`, `node_modules` and non-journal files cheap to walk. A
        // filesystem that answers `DT_UNKNOWN` makes `std` fall back to the
        // `lstat` this used to do unconditionally, so the floor is unchanged.
        let Ok(kind) = entry.file_type() else {
            self.warn(format!("{id} could not be read and was skipped"));
            return;
        };

        if kind.is_symlink() {
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

        if kind.is_dir() {
            if self.spec.skip_dirs.contains(&name) {
                return;
            }
            if depth + 1 > self.spec.max_depth {
                self.truncated = true;
                return;
            }
            subdirs.push(path.to_path_buf());
            return;
        }

        let named = (self.spec.name_ok)(name);
        if !kind.is_file() {
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

        // Belt and braces after the symlink refusal above. A `..` cannot appear
        // in a `read_dir` name and a non-symlink cannot leave the tree, so this
        // should be unreachable — and it costs one `starts_with` to keep the
        // containment claim resting on the shared guard rather than on that
        // argument being right.
        let Some(resolved) = parse::confine(path, &self.root) else {
            self.warn(format!(
                "{id} resolves outside the journal's own directory and was skipped"
            ));
            return;
        };

        // THE `stat`, and the only one: an entry that reached here is one the
        // caller is about to be told about, so `size_bytes` and the recorded
        // `(dev, ino)` describe the same call that admitted it. `DirEntry`'s
        // own, which does not traverse a link, for the same reason the
        // classification above used `file_type`.
        let Ok(meta) = entry.metadata() else {
            self.warn(format!("{id} could not be read and was skipped"));
            return;
        };

        let file = (self.describe)(id, name, resolved, &meta);
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
/// below `root` or has any component that is not a plain UTF-8 name.
///
/// Requiring every component to be [`Component::Normal`] is a guard, not
/// tidiness: it is what makes it impossible for a `.`, a `..`, a root or a
/// Windows prefix to ever appear inside an id, and therefore impossible for a
/// well-formed id to mean anything other than "this file, below the root".
pub(crate) fn relative_id(root: &Path, path: &Path) -> Option<String> {
    let rest = path.strip_prefix(root).ok()?;
    let parts = rest
        .components()
        .map(|component| match component {
            Component::Normal(name) => name.to_str(),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// A best-effort relative label for an entry that has no id — used only in the
/// warning that says so. Falls back to a fixed word rather than to the absolute
/// path, which is the whole point.
fn lossy_relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).map_or_else(
        |_| "an entry".to_string(),
        |rest| rest.to_string_lossy().into_owned(),
    )
}

/// `(st_dev, st_ino)` — the pair that says "the same file", not "a file with the
/// same name". See [`still_the_same_file`].
#[cfg(unix)]
pub(crate) fn file_identity(meta: &Metadata) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    Some((meta.dev(), meta.ino()))
}

/// No portable inode identity outside Unix. Recorded as `None`, which
/// [`still_the_same_file`] treats as "check what can be checked" — the
/// regular-file test alone — rather than as a failure that would make the
/// feature unusable there.
#[cfg(not(unix))]
pub(crate) fn file_identity(_meta: &Metadata) -> Option<(u64, u64)> {
    None
}

/// Re-`symlink_metadata` `path` and require it is still a regular file with the
/// `(dev, ino)` recorded at scan time.
///
/// **Called immediately before a write**, and one half of a pair — the caller
/// re-reads and re-fingerprints the bytes too, because a freed inode number can
/// be handed straight back to the next create. `identity` of `None` degrades
/// this to the regular-file check alone, which is weaker, and honestly so: it
/// can still refuse a name that became a link or a device, but not one that
/// became a different regular file.
pub(crate) fn still_the_same_file(path: &Path, identity: Option<(u64, u64)>) -> bool {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return false;
    };
    meta.file_type().is_file() && identity == file_identity(&meta)
}

// ---------------------------------------------------------------------------
// Tests — the pure helper. The guards need a real filesystem and are exercised
// against one in `tests/rules_security.rs` and `tests/projection_files.rs`,
// which is where they can actually be reached.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_relative_forward_slash_paths_of_plain_components() {
        let root = Path::new("/j");
        assert_eq!(
            relative_id(root, Path::new("/j/plans/p.journal")).as_deref(),
            Some("plans/p.journal")
        );
        assert_eq!(
            relative_id(root, Path::new("/j/import/2026/b.rules")).as_deref(),
            Some("import/2026/b.rules")
        );
        assert_eq!(relative_id(root, Path::new("/j")), None, "the root itself");
        assert_eq!(relative_id(root, Path::new("/other/p.journal")), None);
        assert_eq!(relative_id(root, Path::new("/other/b.rules")), None);
    }
}
