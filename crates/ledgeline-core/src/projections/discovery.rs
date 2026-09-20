//! Finding the `*.journal` files a projection could be loaded from or saved to
//! (plan 22, Phase 3) — and deciding, once, which files the Projections tab is
//! ever allowed to touch.
//!
//! This is [`crate::rules::discovery`] rebuilt for a different extension and a
//! different listing, and the *guards* are deliberately identical: same root
//! ([`crate::parse::include_root_for`]), same containment test
//! ([`crate::parse::confine`]), same refusal of every symlink, same depth /
//! file / entry caps, same [`SKIP_DIRS`] and hidden-entry skip, same
//! re-scan-per-request-never-cache policy, same "never echo an absolute path"
//! rule. Where the two differ is stated below; everywhere else, read that
//! module's docs, which argue each guard at length.
//!
//! # What differs from the rules scan
//!
//! - **The extension is `.journal`, not `.rules`**, and that is the ask's:
//!   *"Really any journal file with budget-like entries could be used."* A
//!   projection is an ordinary journal file; nothing about it is a new format.
//! - **The listing is grouped.** [`DiscoveredProjection::is_projection`] is true
//!   for a `projection-*.journal`, and only those get their header read. Reading
//!   every journal's head to draw a picker is work nobody asked for, and on a
//!   tree whose main journal is 200 MB it is work that shows.
//! - **The header read is bounded and shallow.** [`MAX_HEADER_BYTES`] of the
//!   file, scanned for `; projection:` / `; created:` / `; updated:` in the
//!   leading comment block. No parse, no fingerprint: the listing says what is
//!   there, and `GET /api/projections/{id}` is what opens one.
//! - **No `revision` in the listing.** The rules scan parses each file anyway,
//!   so fingerprinting it there is free; here it would mean reading every
//!   journal in the tree in full to draw a list. The revision is taken by the
//!   route that reads the file, which is the only place it is used.
//!
//! # What does NOT differ, and must not
//!
//! [`Discovery::resolve`] is exact string equality against a set scanned in
//! this request; `root.join(id)` happens in exactly one place,
//! [`Discovery::resolve_new`], under every guard that function documents. A
//! [`ProjectionPath`] has no public constructor, so "you may only write to a
//! file discovery returned" is a property of the type system rather than of
//! code review.
//!
//! # This module does not know about `include`
//!
//! Decision 5 of the plan — *saving is always Save As, and never to anything
//! reachable from the main journal* — is **not** enforced here, because a scan
//! of a directory tree has no idea which of the files in it the journal
//! includes. It is enforced at the HTTP layer, which holds the parsed journal
//! and therefore its `source_files`. That split is deliberate and is stated in
//! both places: a guard nobody can find is a guard that gets removed.

use crate::parse;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

// ---------------------------------------------------------------------------
// Budgets
// ---------------------------------------------------------------------------

/// How far below the scan root the walk descends. The root itself is depth 0.
/// The rules scan's number, for the rules scan's reasons.
const MAX_PROJECTION_DEPTH: usize = 8;

/// How many journal files are RETURNED.
///
/// The same 200 the rules scan returns, and it is a tighter fit here than it
/// looks: a journal split one file per year for twenty years, with a handful of
/// projections beside it, is thirty. A tree with more than 200 `.journal` files
/// in it is one where a picker is the wrong UI anyway, and `truncated` says so
/// rather than showing a subset that looks complete.
const MAX_PROJECTION_FILES: usize = 200;

/// How many directory entries are EXAMINED — **the load-bearing bound**, for
/// the reason `rules::discovery` argues: a journal in `$HOME` would otherwise
/// turn one scan into a full-disk walk, and no skip list prevents that in
/// general.
const MAX_SCAN_ENTRIES: usize = 20_000;

/// How many scan-level warnings are kept, before one path-free note says there
/// were more. These strings reach a dialog.
const MAX_SCAN_WARNINGS: usize = 100;

/// How much of a `projection-*.journal` is read to find its header.
///
/// The header is the leading comment block, which is four lines in a file
/// Ledgeline wrote. 4 KiB is a hundred lines of comments, so it reaches the
/// header of any file a human wrote by hand too — and it bounds the listing's
/// I/O at `MAX_PROJECTION_FILES × 4 KiB` however large the journals are.
///
/// Unlike the rules scan's `MAX_RULES_BYTES` this is **not** a refusal: a
/// multi-megabyte journal is the normal case here, not a suspicious one. It is
/// simply where the read stops.
const MAX_HEADER_BYTES: u64 = 4096;

/// How long a header value may be, in `char`s.
///
/// A display name goes in a picker and in a filename; one that is a kilobyte
/// long is not a name. Clipping rather than refusing, because the file still
/// has to open — the name is display, and the id is what identifies the file.
const MAX_HEADER_VALUE_CHARS: usize = 200;

/// Directory names never descended into. A performance courtesy, not a security
/// control — [`MAX_SCAN_ENTRIES`] is the control. Restated rather than shared
/// with `rules::discovery`: the two scans are free to diverge about what is
/// worth skipping, and a shared constant would make one of them wrong quietly.
const SKIP_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "vendor",
    "dist",
    "build",
    "__pycache__",
];

/// The extension a scanned file must have.
const JOURNAL_SUFFIX: &str = ".journal";

/// The filename prefix that marks a file as one of ours.
///
/// Only a prefix match, and deliberately only display-affecting: a file without
/// it still loads and still saves. The plan's own words — *"a file without it
/// still loads … and takes its name from its filename"* — are about the
/// `; projection:` marker, and this is the same policy one level out.
const PROJECTION_PREFIX: &str = "projection-";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// An absolute path that a [`Discovery`] produced.
///
/// The field is private and there is **no public constructor**, so a write path
/// that takes a `&ProjectionPath` can only ever be handed one that was scanned
/// for. Same type, same argument, as `rules::RulesPath`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionPath(PathBuf);

impl ProjectionPath {
    /// The path, for the one caller that has to open the file.
    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

/// One `*.journal` file found in the journal's directory tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredProjection {
    /// The relative path from the scan root, forward-slash separated. This IS
    /// the id, and resolution is string equality against it.
    pub id: String,
    /// Display fallback: the file name with `.journal` stripped and a leading
    /// `projection-` removed, so `plans/projection-series-a.journal` shows as
    /// `series-a`. Superseded by [`name`](Self::name) when the file has one.
    pub label: String,
    /// Whether the name matches `projection-*.journal` — the group the listing
    /// shows first, and the only files whose header is read.
    pub is_projection: bool,
    /// `; projection: <name>` from the header, for a `projection-*` file that
    /// has one. `None` for every other file, and for one whose header says
    /// nothing — the marker is optional, by decision.
    pub name: Option<String>,
    /// `; created: <date>` from the header, verbatim and unvalidated. The save
    /// path preserves whatever is here rather than re-deriving it.
    pub created: Option<String>,
    /// `; updated: <date>` from the header, verbatim and unvalidated.
    pub updated: Option<String>,
    /// The file's size as of the scan's `stat`. Display only.
    pub size_bytes: u64,
    /// The absolute path, unforgeable and inaccessible outside this module.
    path: ProjectionPath,
    /// `(st_dev, st_ino)` as of the scan, or `None` where there is no such
    /// identity. See [`DiscoveredProjection::identity_unchanged`].
    identity: Option<(u64, u64)>,
}

impl DiscoveredProjection {
    /// The absolute path, for the one caller that has to open the file.
    #[must_use]
    pub fn path(&self) -> &ProjectionPath {
        &self.path
    }

    /// Re-`symlink_metadata` the target and require it is still a regular file
    /// with the `(dev, ino)` recorded at scan time. **Called immediately before
    /// a write**, and one half of a pair — the caller re-reads and
    /// re-fingerprints the bytes too, because a freed inode number can be
    /// handed straight back to the next create. `rules::DiscoveredRules` has
    /// the long version of this argument.
    #[must_use]
    pub fn identity_unchanged(&self) -> bool {
        let Ok(meta) = std::fs::symlink_metadata(&self.path.0) else {
            return false;
        };
        meta.file_type().is_file() && self.identity == file_identity(&meta)
    }
}

/// Why a **new** projection file may not be created at an id.
///
/// One variant per guard in [`Discovery::resolve_new`]. The split matters to a
/// caller: two of them are about places this module declined to look and must
/// be reported indistinguishably, while [`Exists`](Self::Exists) is about the
/// set `GET /api/projections` already publishes and is safe to report as
/// itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CreateRefusal {
    /// Not a shape a scan could have produced, or one it would skip: a
    /// traversal, a hidden component, a [`SKIP_DIRS`] directory, a name not
    /// ending `.journal`.
    Malformed,
    /// Resolves outside the journal's own directory.
    OutsideRoot,
    /// The directory it would go in is not there, is not a directory, or is a
    /// symlink. **No directory is ever created** — offering the directories the
    /// scan found is the dialog's job; making new ones is the user's.
    DirectoryMissing,
    /// Something is already at that name.
    Exists,
}

/// The result of one scan.
#[derive(Debug, Clone)]
pub struct Discovery {
    /// The canonical scan root. **Private and never leaves this module.**
    root: PathBuf,
    /// The files found, sorted `projection-*` first and by id within each
    /// group, so two scans of an unchanged tree produce identical output and a
    /// picker needs no sort of its own.
    pub files: Vec<DiscoveredProjection>,
    /// A cap was hit and the list is incomplete. Surfaced so a user is never
    /// silently shown a subset.
    pub truncated: bool,
    /// What the walk skipped and why, each naming a **relative** path only.
    /// Bounded by [`MAX_SCAN_WARNINGS`].
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Find every `*.journal` file in the open journal's own directory tree.
///
/// `main_journal_file` is `Journal::source_files[0]`, canonicalized and always
/// first; its *directory* is the scan root, so the projections surface and the
/// `include` guard confine to exactly the same place.
///
/// Infallible, like the rules scan and like the parser: an unreadable
/// directory, a symlink and a non-regular file each become a warning naming a
/// RELATIVE path. Showing the user what is there is the whole point of the
/// screen, and one unreadable directory is not a reason to show nothing.
#[must_use]
pub fn discover(main_journal_file: &Path) -> Discovery {
    Scan::new(parse::include_root_for(
        &main_journal_file.to_string_lossy(),
    ))
    .run()
}

impl Discovery {
    /// The **one** id → path resolution for projections.
    ///
    /// Exact string equality against this scan's set, never `root.join(id)`:
    /// the client's string is only ever *compared*, so the path it selects is
    /// one this module built from a `read_dir` name moments earlier. Callers
    /// scan, resolve and write within one request; a stale id simply misses.
    #[must_use]
    pub fn resolve(&self, id: &str) -> Option<&DiscoveredProjection> {
        self.files.iter().find(|file| file.id == id)
    }

    /// The scan root's final path component, for a heading. **Not** the path.
    #[must_use]
    pub fn root_label(&self) -> String {
        self.root.file_name().map_or_else(
            || "journal".to_string(),
            |name| name.to_string_lossy().into_owned(),
        )
    }

    /// The directories the scan descended into, as relative ids, so a Save As
    /// dialog can offer somewhere to put a file without the user typing a path
    /// — and without this module ever handing out an absolute one.
    ///
    /// The root is `""`, the empty string, which is what an id with no
    /// directory part joins to. Sorted, deduplicated, and derived from the ids
    /// already in [`files`](Self::files) rather than recorded during the walk:
    /// a directory with no journal in it is not somewhere a projection wants to
    /// go, and offering it would be offering a place the scan will not list the
    /// result from.
    #[must_use]
    pub fn directories(&self) -> Vec<String> {
        let mut dirs: Vec<String> = self
            .files
            .iter()
            .map(|file| match file.id.rfind('/') {
                Some(cut) => file.id[..cut].to_string(),
                None => String::new(),
            })
            .collect();
        dirs.push(String::new());
        dirs.sort();
        dirs.dedup();
        dirs
    }

    /// Where a **new** projection file called `id` would go — the one path in
    /// this module built by joining a caller's string onto the root.
    ///
    /// The guards, in order, are `rules::Discovery::resolve_new`'s, and that
    /// function's docs argue each one:
    ///
    /// 1. **Shape** — before any filesystem call, and deliberately a *second*
    ///    copy of the question the HTTP layer's own `validate_id` asks.
    /// 2. **Discoverability** — no component may be hidden or in
    ///    [`SKIP_DIRS`], because the scan skips those, and creating a file the
    ///    scan will never list writes something the user cannot then open.
    /// 3. **Confinement** — [`crate::parse::confine`], the same function
    ///    `include` is held to.
    /// 4. **A real parent directory** — `symlink_metadata`, so a symlinked
    ///    directory is refused rather than followed. No directory is ever
    ///    created.
    /// 5. **Nothing there already.**
    ///
    /// Guard 5 is a courtesy that produces a good error message and **is not
    /// what makes the create safe**: it expires the moment it returns. The
    /// write itself is `O_EXCL`, and that open is what produces the guarantee.
    ///
    /// # Errors
    /// [`CreateRefusal`], one variant per guard above.
    pub fn resolve_new(&self, id: &str) -> Result<ProjectionPath, CreateRefusal> {
        let parts: Vec<&str> = id.split('/').collect();
        let well_formed = !id.is_empty()
            && parts.len() <= MAX_PROJECTION_DEPTH + 1
            && parts.iter().all(|part| {
                !part.is_empty()
                    && *part != "."
                    && *part != ".."
                    && !part.starts_with('.')
                    && !part.contains('\\')
                    && !part.contains(':')
                    && !part.chars().any(|c| c.is_ascii_control())
            })
            && parts[..parts.len() - 1]
                .iter()
                .all(|part| !SKIP_DIRS.contains(part))
            && parts.last().is_some_and(|name| is_journal_name(name));
        if !well_formed {
            return Err(CreateRefusal::Malformed);
        }

        // THE join. Everything above is what earns it.
        let candidate = self.root.join(id);
        let Some(resolved) = parse::confine(&candidate, &self.root) else {
            return Err(CreateRefusal::OutsideRoot);
        };
        // **A symlink anywhere in the id is refused**, and this is the test
        // that does it. `confine` CANONICALIZES, so a `linked/p.journal` whose
        // `linked` is a symlink into the root comes back resolved to the
        // target and passes containment — which would put the file somewhere
        // the id does not name, and therefore somewhere the scan lists under a
        // DIFFERENT id, so the user could not open what they had just created.
        //
        // The root is already canonical and every component of a well-formed
        // id is a plain name, so the two paths are equal exactly when no link
        // (and no case-folding filesystem) rewrote one of them. Comparing them
        // fails closed, which is the right direction for a create.
        //
        // `rules::Discovery::resolve_new` has the same latent gap and is left
        // alone here: changing it is a change to a different feature's write
        // surface, and this comment is the record that it wants the same line.
        if resolved != candidate {
            return Err(CreateRefusal::DirectoryMissing);
        }
        let Some(parent) = resolved.parent() else {
            return Err(CreateRefusal::DirectoryMissing);
        };
        // `symlink_metadata`, so a parent that is itself a link is refused
        // rather than followed — belt and braces after the comparison above.
        match std::fs::symlink_metadata(parent) {
            Ok(meta) if meta.file_type().is_dir() => {}
            _ => return Err(CreateRefusal::DirectoryMissing),
        }
        match std::fs::symlink_metadata(&resolved) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(ProjectionPath(resolved))
            }
            // Present, or unreadable in a way that is not "absent". Either way
            // this is not a name a create may have.
            _ => Err(CreateRefusal::Exists),
        }
    }
}

// ---------------------------------------------------------------------------
// The walk
// ---------------------------------------------------------------------------

/// One scan in progress.
///
/// Iterative with an explicit stack, never recursive: the depth cap bounds a
/// well-behaved tree, but recursion turns a *mistake* in that bound into a
/// stack overflow, which is a `SIGABRT` and not a catchable panic.
struct Scan {
    root: PathBuf,
    files: Vec<DiscoveredProjection>,
    warnings: Vec<String>,
    truncated: bool,
    examined: usize,
}

impl Scan {
    fn new(root: PathBuf) -> Self {
        Self {
            root,
            files: Vec::new(),
            warnings: Vec::new(),
            truncated: false,
            examined: 0,
        }
    }

    /// Walk the tree, depth-first, visiting each directory's entries in name
    /// order — so that when a cap trips, *which* files survive is reproducible
    /// rather than `read_dir`-order-dependent.
    fn run(mut self) -> Discovery {
        let mut stack = vec![(self.root.clone(), 0usize)];
        while let Some((dir, depth)) = stack.pop() {
            let Some(children) = self.read_dir(&dir) else {
                continue;
            };
            let mut subdirs = Vec::new();
            for path in children {
                // Both budgets are checked in exactly one place, before any
                // work is done for the entry.
                if self.examined >= MAX_SCAN_ENTRIES || self.files.len() >= MAX_PROJECTION_FILES {
                    self.truncated = true;
                    return self.finish();
                }
                self.examined += 1;
                self.visit(&path, depth, &mut subdirs);
            }
            // Reversed, so the stack pops them back in name order.
            stack.extend(subdirs.into_iter().rev().map(|dir| (dir, depth + 1)));
        }
        self.finish()
    }

    fn finish(mut self) -> Discovery {
        // `projection-*` first, then by id. The grouping the plan asks the
        // listing for, done once here so every consumer agrees about it.
        self.files
            .sort_by(|a, b| b.is_projection.cmp(&a.is_projection).then(a.id.cmp(&b.id)));
        Discovery {
            root: self.root,
            files: self.files,
            truncated: self.truncated,
            warnings: self.warnings,
        }
    }

    /// This directory's entries, sorted, and never more than the remaining
    /// entry budget. The `take` is not decoration: collecting a directory of a
    /// million names before checking the budget would let one directory defeat
    /// [`MAX_SCAN_ENTRIES`] by allocating instead of by walking.
    fn read_dir(&mut self, dir: &Path) -> Option<Vec<PathBuf>> {
        match std::fs::read_dir(dir) {
            Ok(entries) => {
                let mut paths: Vec<PathBuf> = entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.path())
                    .take(MAX_SCAN_ENTRIES.saturating_sub(self.examined) + 1)
                    .collect();
                paths.sort();
                Some(paths)
            }
            Err(_) => {
                // The `io::Error` is deliberately not quoted: it is very
                // probably path-free, but "very probably" is not a property
                // this module can assert about every platform's error text,
                // and this string reaches a dialog.
                let message = match relative_id(&self.root, dir) {
                    Some(id) => format!(
                        "the directory {id} could not be read; anything inside it was skipped"
                    ),
                    None => "the journal's own directory could not be read".to_string(),
                };
                self.warn(message);
                None
            }
        }
    }

    /// Classify one directory entry. The order of the checks is the order of
    /// the guarantees.
    fn visit(&mut self, path: &Path, depth: usize, subdirs: &mut Vec<PathBuf>) {
        let (Some(id), Some(name)) = (
            relative_id(&self.root, path),
            path.file_name().and_then(std::ffi::OsStr::to_str),
        ) else {
            // A name that is not valid UTF-8. Skipped rather than lossily
            // converted: two different names can lossily convert to the SAME
            // id, and an id that resolves to the wrong file is a write to the
            // wrong file.
            self.warn(format!(
                "{} has a name that is not valid UTF-8 and was skipped",
                lossy_relative(&self.root, path)
            ));
            return;
        };

        // `symlink_metadata`, never `metadata`: the point is to see the link
        // rather than what it points at.
        let Ok(meta) = std::fs::symlink_metadata(path) else {
            self.warn(format!("{id} could not be read and was skipped"));
            return;
        };
        let kind = meta.file_type();

        if kind.is_symlink() {
            self.warn(format!(
                "{id} is a symbolic link and was skipped; projections are only read from and \
                 written to real files inside the journal's own directory"
            ));
            return;
        }

        // A leading dot, on a directory OR on a file. A hidden entry is one the
        // user's own file browser does not show them, and for directories this
        // is what keeps `.git/` and `.direnv/` out. Silent, like `SKIP_DIRS`:
        // a policy skip is not a problem to report.
        if name.starts_with('.') {
            return;
        }

        if kind.is_dir() {
            if SKIP_DIRS.contains(&name) {
                return;
            }
            if depth + 1 > MAX_PROJECTION_DEPTH {
                self.truncated = true;
                return;
            }
            subdirs.push(path.to_path_buf());
            return;
        }

        let named_journal = is_journal_name(name);
        if !kind.is_file() {
            // FIFOs, devices, sockets. A FIFO is the one that bites: a `read`
            // on one with no writer blocks forever, which would hang the
            // request that asked for the list rather than merely fail it.
            if named_journal {
                self.warn(format!("{id} is not a regular file and was skipped"));
            }
            return;
        }
        if !named_journal {
            return;
        }

        // Belt and braces after the symlink refusal above: a `..` cannot appear
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

        self.files.push(describe(id, name, resolved, &meta));
    }

    /// Record a scan-level warning, up to [`MAX_SCAN_WARNINGS`] of them plus
    /// one path-free note saying there were more.
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
// One file
// ---------------------------------------------------------------------------

/// Summarize one admitted file, reading its header only when it is one of ours.
fn describe(
    id: String,
    name: &str,
    path: PathBuf,
    meta: &std::fs::Metadata,
) -> DiscoveredProjection {
    let is_projection = is_projection_name(name);
    let header = if is_projection {
        read_header(&path)
    } else {
        Header::default()
    };
    DiscoveredProjection {
        label: label_for(&id),
        is_projection,
        name: header.name,
        created: header.created,
        updated: header.updated,
        size_bytes: meta.len(),
        path: ProjectionPath(path),
        identity: file_identity(meta),
        id,
    }
}

/// The three header comments a listing shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Header {
    /// `; projection: <name>`.
    pub(super) name: Option<String>,
    /// `; created: <date>`.
    pub(super) created: Option<String>,
    /// `; updated: <date>`.
    pub(super) updated: Option<String>,
}

/// Read the leading comment block of a file and pull the header out of it.
///
/// Unreadable, non-UTF-8 or headerless all answer the same empty [`Header`]:
/// the marker is optional by decision, so "no header" is an ordinary state and
/// not a failure to report. An over-long value is clipped rather than dropped
/// — a name is display, and the id is what identifies the file.
fn read_header(path: &Path) -> Header {
    let Ok(file) = std::fs::File::open(path) else {
        return Header::default();
    };
    let mut bytes = Vec::new();
    if file.take(MAX_HEADER_BYTES).read_to_end(&mut bytes).is_err() {
        return Header::default();
    }
    // Lossy: a journal whose *body* is not UTF-8 can still have an ASCII
    // header, and refusing to show its name would be refusing to list it.
    // Nothing here is written back, so a replacement character costs a display
    // glyph and nothing else.
    parse_header(&String::from_utf8_lossy(&bytes))
}

/// Pull `; projection:` / `; created:` / `; updated:` out of a file's LEADING
/// comment block.
///
/// Leading, and it stops at the first line that is neither a comment nor blank:
/// a `; projection: …` written halfway down a file, after a transaction, is a
/// note about that transaction and not this file's name. hledger's own comment
/// characters are `;` and `#`, and both are accepted, because a user who wrote
/// their header with `#` did not mean something different by it.
pub(super) fn parse_header(text: &str) -> Header {
    let mut header = Header::default();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some(comment) = trimmed.strip_prefix([';', '#']) else {
            break;
        };
        let comment = comment.trim();
        let Some((key, value)) = comment.split_once(':') else {
            continue;
        };
        let value = clip(value.trim());
        if value.is_empty() {
            continue;
        }
        // First wins. A file with two `; projection:` lines is a file someone
        // edited by hand, and the first is the one a reader sees at the top.
        let slot = match key.trim().to_ascii_lowercase().as_str() {
            "projection" => &mut header.name,
            "created" => &mut header.created,
            "updated" => &mut header.updated,
            _ => continue,
        };
        if slot.is_none() {
            *slot = Some(value);
        }
    }
    header
}

/// A header value, clipped to [`MAX_HEADER_VALUE_CHARS`] and stripped of ASCII
/// control characters.
///
/// The control strip is not decoration: this string reaches a picker, a
/// filename slug and an error message, and a `\r` or an ANSI escape in any of
/// those is a display bug at best.
fn clip(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_ascii_control())
        .take(MAX_HEADER_VALUE_CHARS)
        .collect::<String>()
        .trim()
        .to_string()
}

// ---------------------------------------------------------------------------
// Names
// ---------------------------------------------------------------------------

/// Whether `name` names a journal file: it ends in `.journal`,
/// ASCII-case-insensitively, and is more than just `.journal` (which is a
/// dotfile, and is skipped anyway).
///
/// Compared as BYTES: slicing a `&str` at a fixed offset from the end could
/// land mid-code-point and panic, and a filename is attacker-chosen input on
/// this path.
#[must_use]
pub fn is_journal_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    let suffix = JOURNAL_SUFFIX.as_bytes();
    bytes.len() > suffix.len() && bytes[bytes.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
}

/// Whether `name` is one Ledgeline's Save As dialog would have written:
/// `projection-<slug>.journal`. Display grouping only — a file without the
/// prefix still loads and still saves.
fn is_projection_name(name: &str) -> bool {
    is_journal_name(name) && name.to_ascii_lowercase().starts_with(PROJECTION_PREFIX)
}

/// The display fallback for a file with no `; projection:` header: the file
/// name with `.journal` stripped and a leading `projection-` removed.
///
/// `pub` for the create path, which has to label a file no scan has produced
/// yet — sharing this rather than re-deriving it there is what stops a file
/// being titled differently from the same file once it is on disk.
#[must_use]
pub fn label_for(id: &str) -> String {
    let name = id.rsplit('/').next().unwrap_or(id);
    let stem = name
        .get(..name.len().saturating_sub(JOURNAL_SUFFIX.len()))
        .filter(|_| is_journal_name(name))
        .unwrap_or(name);
    stem.strip_prefix(PROJECTION_PREFIX)
        .unwrap_or(stem)
        .to_string()
}

/// The filename a scenario called `name` is saved as: `projection-<slug>.journal`.
///
/// The slug is `name` lowercased with runs of non-alphanumerics collapsed to
/// `-` and the ends trimmed, exactly as the plan specifies. A name that slugs
/// to nothing — punctuation, or a script with no ASCII alphanumerics — answers
/// `None` rather than `projection-.journal`, which is a dotfile-adjacent name
/// the scan would list and nobody chose.
///
/// Non-ASCII alphanumerics are **dropped**, not transliterated: a slug is a
/// filename on somebody else's filesystem too, and `char::is_alphanumeric`
/// would let a name normalize differently on two machines. The display name is
/// the user's own text and keeps every character.
#[must_use]
pub fn slug_filename(name: &str) -> Option<String> {
    let mut slug = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        return None;
    }
    // Bounded for the same reason `clip` is: this becomes a path component,
    // and every filesystem has a limit on one.
    let slug: String = slug.chars().take(MAX_HEADER_VALUE_CHARS).collect();
    Some(format!(
        "{PROJECTION_PREFIX}{}{JOURNAL_SUFFIX}",
        slug.trim_end_matches('-')
    ))
}

/// `path` relative to `root`, forward-slash separated, or `None` if it is not
/// below `root` or has any component that is not a plain UTF-8 name.
///
/// Requiring every component to be [`Component::Normal`] is a guard, not
/// tidiness: it is what makes it impossible for a `.`, a `..`, a root or a
/// Windows prefix to appear inside an id, and therefore impossible for a
/// well-formed id to mean anything other than "this file, below the root".
fn relative_id(root: &Path, path: &Path) -> Option<String> {
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

/// `(st_dev, st_ino)` — the pair that says "the same file", not "a file with
/// the same name".
#[cfg(unix)]
fn file_identity(meta: &std::fs::Metadata) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    Some((meta.dev(), meta.ino()))
}

/// No portable inode identity outside Unix. `None` degrades
/// [`DiscoveredProjection::identity_unchanged`] to the regular-file check
/// alone, which is weaker and honestly so.
#[cfg(not(unix))]
fn file_identity(_meta: &std::fs::Metadata) -> Option<(u64, u64)> {
    None
}

// ---------------------------------------------------------------------------
// Tests
//
// The pure helpers here; the guards that need a real filesystem — the caps, the
// symlink refusal, the hidden skip and containment — are exercised against one
// in `crates/ledgeline-core/tests/projection_files.rs`, which is the only place
// they can actually be reached.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn journal_names_are_matched_case_insensitively_and_never_bare() {
        assert!(is_journal_name("2026.journal"));
        assert!(is_journal_name("a.JOURNAL"));
        assert!(is_journal_name("projection-x.Journal"));
        assert!(
            !is_journal_name(".journal"),
            "a bare dotfile is not a journal file"
        );
        assert!(!is_journal_name("journal"));
        assert!(!is_journal_name("x.journal.bak"));
        assert!(!is_journal_name("x.ledger"));
        assert!(!is_journal_name(""));
    }

    #[test]
    fn a_multibyte_name_shorter_than_the_suffix_check_does_not_panic() {
        // `a€bcde` is eight bytes whose eighth-from-last is inside the `€`.
        // Indexing there would panic, and a filename is attacker-chosen input.
        for name in ["a€bcde", "€", "€€", "€.journal", "日本語の元帳"] {
            let _ = is_journal_name(name);
            let _ = label_for(name);
            let _ = is_projection_name(name);
        }
        assert!(is_journal_name("a€.journal"));
        assert_eq!(label_for("a€.journal"), "a€");
    }

    #[test]
    fn labels_strip_the_extension_and_the_projection_prefix() {
        assert_eq!(label_for("plans/projection-series-a.journal"), "series-a");
        assert_eq!(label_for("2026.journal"), "2026");
        assert_eq!(label_for("PROJECTION-X.JOURNAL"), "PROJECTION-X");
        assert_eq!(
            label_for("odd.name.journal"),
            "odd.name",
            "only the final extension is a suffix"
        );
    }

    #[test]
    fn the_projection_group_is_a_case_insensitive_prefix_on_a_journal_name() {
        assert!(is_projection_name("projection-a.journal"));
        assert!(is_projection_name("Projection-A.Journal"));
        assert!(!is_projection_name("projections-a.journal"));
        assert!(
            !is_projection_name("projection-a.rules"),
            "the prefix alone is not enough"
        );
    }

    #[test]
    fn a_slug_collapses_runs_and_trims_the_ends() {
        assert_eq!(
            slug_filename("Series A with a hiring ramp").as_deref(),
            Some("projection-series-a-with-a-hiring-ramp.journal")
        );
        assert_eq!(
            slug_filename("  ***Plan B!!! 2027  ").as_deref(),
            Some("projection-plan-b-2027.journal")
        );
        assert_eq!(
            slug_filename("a/../../etc/passwd").as_deref(),
            Some("projection-a-etc-passwd.journal"),
            "every separator and dot is a non-alphanumeric, so a traversal slugs away"
        );
    }

    #[test]
    fn a_name_with_no_ascii_alphanumerics_has_no_filename() {
        // `projection-.journal` would be a name nobody chose, so the dialog is
        // told to ask again rather than handed one.
        assert_eq!(slug_filename("—"), None);
        assert_eq!(slug_filename("   "), None);
        assert_eq!(slug_filename(""), None);
        assert_eq!(slug_filename("日本語"), None);
    }

    #[test]
    fn the_header_is_read_from_the_leading_comment_block_only() {
        let header = parse_header(
            "; Ledgeline projection\n\
             ; projection: Series A with a hiring ramp\n\
             ; created: 2026-09-18\n\
             ; updated: 2026-09-20\n\
             \n\
             ~ monthly  projection\n\
             ; projection: not this one\n",
        );
        assert_eq!(header.name.as_deref(), Some("Series A with a hiring ramp"));
        assert_eq!(header.created.as_deref(), Some("2026-09-18"));
        assert_eq!(header.updated.as_deref(), Some("2026-09-20"));
    }

    #[test]
    fn a_header_after_a_directive_is_not_this_file_s_header() {
        // The scan stops at the first non-comment, non-blank line: a
        // `; projection:` further down is a note on what precedes it.
        let header = parse_header("account assets:cash\n; projection: Nope\n");
        assert_eq!(header.name, None);
    }

    #[test]
    fn a_hash_comment_is_a_comment_too() {
        let header = parse_header("# projection: Hashed\n");
        assert_eq!(header.name.as_deref(), Some("Hashed"));
    }

    #[test]
    fn header_values_are_clipped_and_stripped_of_control_characters() {
        let long = "x".repeat(MAX_HEADER_VALUE_CHARS + 50);
        let header = parse_header(&format!("; projection: {long}\n"));
        assert_eq!(
            header.name.as_deref().map(str::len),
            Some(MAX_HEADER_VALUE_CHARS)
        );

        let header = parse_header("; projection: a\u{1b}[31mb\n");
        assert_eq!(header.name.as_deref(), Some("a[31mb"));
    }

    #[test]
    fn the_first_of_a_repeated_key_wins() {
        let header = parse_header("; projection: first\n; projection: second\n");
        assert_eq!(header.name.as_deref(), Some("first"));
    }

    #[test]
    fn a_header_with_no_marker_reads_as_nothing_rather_than_failing() {
        // "A file without it still loads … and takes its name from its
        // filename." An empty header is an ordinary state.
        assert_eq!(parse_header(""), Header::default());
        assert_eq!(parse_header("; just a note\n"), Header::default());
        assert_eq!(parse_header("; projection:   \n"), Header::default());
    }

    #[test]
    fn ids_are_relative_forward_slash_paths_of_plain_components() {
        let root = Path::new("/j");
        assert_eq!(
            relative_id(root, Path::new("/j/plans/p.journal")).as_deref(),
            Some("plans/p.journal")
        );
        assert_eq!(relative_id(root, Path::new("/j")), None, "the root itself");
        assert_eq!(relative_id(root, Path::new("/other/p.journal")), None);
    }
}
