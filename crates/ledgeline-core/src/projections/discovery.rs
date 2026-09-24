//! Finding the `*.journal` files a projection could be loaded from or saved to
//! (plan 22, Phase 3) — and deciding, once, which files the Projections tab is
//! ever allowed to touch.
//!
//! The walk itself is [`crate::dirscan`], which [`crate::rules::discovery`]
//! also drives: same root ([`crate::parse::include_root_for`]), same
//! containment test ([`crate::parse::confine`]), same refusal of every symlink,
//! same entry and warning budgets, same hidden-entry skip, same "never echo an
//! absolute path" rule — not because two modules agree about them but because
//! there is only one copy. The re-scan-per-request-never-cache policy is this
//! module's, and is stated below. What this file adds is the four things that
//! are genuinely different, and the summary a projection listing renders.
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
//! - **The header read is bounded, shallow, and skippable.**
//!   [`MAX_HEADER_BYTES`] of the file, scanned for `; projection:` /
//!   `; created:` / `; updated:` in the leading comment block. No parse, no
//!   fingerprint: the listing says what is there, and
//!   `GET /api/projections/{id}` is what opens one. A caller that only means to
//!   [`Discovery::resolve`] one id skips it entirely —
//!   [`Discovery::without_headers`].
//! - **No `revision` in the listing.** The rules scan parses each file anyway,
//!   so fingerprinting it there is free; here it would mean reading every
//!   journal in the tree in full to draw a list. The revision is taken by the
//!   route that reads the file, which is the only place it is used.
//!
//! # What does NOT differ, and must not
//!
//! [`Discovery::resolve`] is exact string equality against a set scanned in
//! this request; `root.join(id)` happens in exactly one place,
//! [`crate::parse::resolve_new_in`], under every guard that function documents.
//! A [`ProjectionPath`] has no public constructor, so "you may only write to a
//! file discovery returned" is a property of the type system rather than of
//! code review — and it is a type of this module's own, so a rules scan can
//! never mint one.
//!
//! # This module does not know about `include`
//!
//! Decision 5 of the plan — *saving is always Save As, and never to anything
//! reachable from the main journal* — is **not** enforced here, because a scan
//! of a directory tree has no idea which of the files in it the journal
//! includes. It is enforced at the HTTP layer, which holds the parsed journal
//! and therefore its `source_files`. That split is deliberate and is stated in
//! both places: a guard nobody can find is a guard that gets removed.

use crate::dirscan::{self, ScanSpec};
use crate::parse;
use std::io::Read;
use std::path::{Path, PathBuf};

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
///
/// It is also the reason [`Discovery::without_headers`] exists: bounded is not
/// free, and two of the three callers never look at what this buys.
const MAX_HEADER_BYTES: u64 = 4096;

/// How long a header value may be, in `char`s.
///
/// A display name goes in a picker and in a filename; one that is a kilobyte
/// long is not a name. Clipping rather than refusing, because the file still
/// has to open — the name is display, and the id is what identifies the file.
const MAX_HEADER_VALUE_CHARS: usize = 200;

/// Directory names never descended into. A performance courtesy, not a security
/// control — the shared scan's entry budget is the control. Stated here rather
/// than shared with `rules::discovery`, which is why [`ScanSpec::skip_dirs`] is
/// a field and not a constant: the two scans are free to diverge about what is
/// worth skipping, and one list for both would make one of them wrong quietly.
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
    /// handed straight back to the next create.
    /// [`dirscan::still_the_same_file`] has the long version of this argument.
    #[must_use]
    pub fn identity_unchanged(&self) -> bool {
        dirscan::still_the_same_file(&self.path.0, self.identity)
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

impl From<parse::NewPathRefusal> for CreateRefusal {
    /// The shared guard chain's refusal, under this module's own name.
    ///
    /// A re-badging and not a translation: the variants are one-for-one, and
    /// the type is separate only so a caller matching on it is told which
    /// feature refused.
    fn from(refusal: parse::NewPathRefusal) -> Self {
        match refusal {
            parse::NewPathRefusal::Malformed => Self::Malformed,
            parse::NewPathRefusal::OutsideRoot => Self::OutsideRoot,
            parse::NewPathRefusal::DirectoryMissing => Self::DirectoryMissing,
            parse::NewPathRefusal::Exists => Self::Exists,
        }
    }
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
    /// What the walk skipped and why, each naming a **relative** path only, and
    /// bounded by the shared scan's warning budget.
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
/// Every `projection-*.journal` has its header read, so the listing can show a
/// display name and its dates.
///
/// Infallible, like the rules scan and like the parser: an unreadable
/// directory, a symlink and a non-regular file each become a warning naming a
/// RELATIVE path. Showing the user what is there is the whole point of the
/// screen, and one unreadable directory is not a reason to show nothing.
#[must_use]
pub fn discover(main_journal_file: &Path) -> Discovery {
    scan_tree(main_journal_file, Headers::Read)
}

/// Whether [`describe`] opens a `projection-*.journal` to read its header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Headers {
    Read,
    Skip,
}

/// The one walk, under both entry points.
fn scan_tree(main_journal_file: &Path, headers: Headers) -> Discovery {
    let root = parse::include_root_for(&main_journal_file.to_string_lossy());
    let spec = ScanSpec {
        name_ok: is_journal_name,
        skip_dirs: SKIP_DIRS,
        max_depth: MAX_PROJECTION_DEPTH,
        max_files: MAX_PROJECTION_FILES,
        symlink_note: "projections are only read from and written to real files inside the \
                       journal's own directory",
    };
    let scanned = dirscan::scan(root, &spec, |id, name, path, meta| {
        describe(id, name, path, meta, headers)
    });
    let mut files = scanned.files;
    // `projection-*` first, then by id. The grouping the plan asks the listing
    // for, done once here so every consumer agrees about it — and so two scans
    // of an unchanged tree produce identical output.
    files.sort_by(|a, b| b.is_projection.cmp(&a.is_projection).then(a.id.cmp(&b.id)));
    Discovery {
        root: scanned.root,
        files,
        truncated: scanned.truncated,
        warnings: scanned.warnings,
    }
}

impl Discovery {
    /// The same scan as [`discover`], the same guards and the same set — but
    /// **no file is opened**, so [`DiscoveredProjection::name`],
    /// [`created`](DiscoveredProjection::created) and
    /// [`updated`](DiscoveredProjection::updated) are all `None`.
    ///
    /// For a caller that scans only in order to [`resolve`](Self::resolve) one
    /// id, which is most of them. `GET` and `PUT` of a single projection each
    /// have to scan the tree (see below) and then use exactly one entry, so
    /// reading a header for the other 199 is an open, a 4 KiB read and a lossy
    /// UTF-8 decode thrown away per file — and the `PUT` did it holding the
    /// process-wide write lock. Only the *listing* ever looks at a header.
    ///
    /// # The scan is still fresh, and that is not negotiable
    ///
    /// This is not a cached scan, and there is no cached scan: the entry
    /// budget, the depth cap and every `read_dir` still happen, every request.
    /// An id resolves against a set built from `read_dir` names moments ago, so
    /// a cached set would be one that no longer describes the disk, and a write
    /// would be authorized by a scan that happened arbitrarily long ago. What
    /// is skipped here is reading the *contents* of files nobody asked about,
    /// which is not a guard and never was.
    #[must_use]
    pub fn without_headers(main_journal_file: &Path) -> Self {
        scan_tree(main_journal_file, Headers::Skip)
    }

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

    /// Where a **new** projection file called `id` would go.
    ///
    /// The join, and the five guards that earn it, are
    /// [`crate::parse::resolve_new_in`]'s — shared with the rules create path,
    /// which is where the argument for each one is written out. This module
    /// supplies the three things that are its own (how deep, which directories
    /// a scan would skip, which names it would list) and wraps the result in
    /// the newtype that makes a write path unable to be handed anything else.
    ///
    /// `plans/22-projections.md` amendment 31 recorded these guards as
    /// deliberate copies "so a change to either belongs in both". There is one
    /// copy now, which is the same decision arrived at by a cheaper route: the
    /// symlinked-parent test that amendment is about was missing from one of
    /// the two for exactly as long as there were two.
    ///
    /// # Errors
    /// [`CreateRefusal`], one variant per guard.
    pub fn resolve_new(&self, id: &str) -> Result<ProjectionPath, CreateRefusal> {
        parse::resolve_new_in(
            &self.root,
            id,
            MAX_PROJECTION_DEPTH,
            SKIP_DIRS,
            is_journal_name,
        )
        .map(ProjectionPath)
        .map_err(CreateRefusal::from)
    }
}

// ---------------------------------------------------------------------------
// One file
// ---------------------------------------------------------------------------

/// Summarize one admitted file, reading its header only when it is one of ours
/// **and** somebody is going to look at it.
///
/// `meta` is the scan's `symlink_metadata`, so `size_bytes` and the recorded
/// identity describe the same `stat` that admitted the file.
fn describe(
    id: String,
    name: &str,
    path: PathBuf,
    meta: &std::fs::Metadata,
    headers: Headers,
) -> DiscoveredProjection {
    let is_projection = is_projection_name(name);
    let header = if is_projection && headers == Headers::Read {
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
        identity: dirscan::file_identity(meta),
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
}
