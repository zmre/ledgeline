//! Finding the `*.rules` files a journal's own directory tree contains
//! (Imports, step 5) — and deciding, once, which files the imports feature is
//! ever allowed to touch. Step 6 adds [`Discovery::preview`], which reads the
//! first few rows of the *data* file a rules file describes.
//!
//! Steps 2-4 built a model that can rewrite a rules file without disturbing a
//! byte it was not asked to change. This module answers the question that comes
//! before that one: *which* files. It is the most security-sensitive part of the
//! feature, because the set it returns is exactly the set a later `PUT` endpoint
//! may overwrite. Everything below is a guard, and each one is load-bearing.
//!
//! # The root is the journal's own directory, and it is not a new decision
//!
//! The scan root is [`crate::parse::include_root_for`] — the main journal file's
//! own directory, canonicalized — which is the *same* root `include` is confined
//! to. Containment is tested with the *same* function, [`crate::parse::confine`].
//! That is deliberate: it means the rules surface can never reach a file the
//! journal itself could not have included, and there is no second, hand-rolled
//! traversal check to get fixed in one place and not the other.
//!
//! # Stricter than `include`: symlinks are refused outright
//!
//! `admit_include` *resolves* a symlink and admits it if it lands inside the
//! root. This scan **skips every symlink**, file or directory, and says so in a
//! warning. The divergence is intentional. A directory walk that follows links
//! has to contend with cycles, with a link that re-enters the tree at a
//! different depth, and with a link whose target is swapped between the walk and
//! the write. Refusing links removes all three at once, and costs nothing real:
//! a rules file reached only through a symlink is not a rules file this feature
//! needs to offer to edit. An `include` has no such luxury — the user wrote the
//! path and expects it followed.
//!
//! # Never echo a resolved path
//!
//! Every warning names a path **relative to the scan root**, and every id is a
//! relative path. No absolute path, and no part of the root, appears in any
//! string this module produces. `parse.rs` already documents why for `include`
//! diagnostics; here it matters more, because a later HTTP layer surfaces these
//! strings verbatim in a user-facing dialog, and a dialog is a fine oracle for
//! "does `/Users/someone/…` exist". [`Discovery::root_label`] exists so a GUI can
//! still write a heading without ever being handed the path.
//!
//! # Ids are strings, and resolution is string equality
//!
//! [`DiscoveredRules::id`] is the forward-slash-separated relative path, and it
//! is the *only* handle a client ever gets. [`Discovery::resolve`] matches it by
//! **exact string equality** against a freshly scanned set; `root.join(id)`
//! appears nowhere in this crate, and must not. Two consequences follow, and
//! both are deliberate:
//!
//! - On a case-insensitive filesystem, asking for `Checking.rules` when the file
//!   is `checking.rules` **misses**. Case-folding the lookup would mean deciding
//!   which spellings name the same file, which is path arithmetic wearing a
//!   different hat.
//! - An id whose components are not all plain, UTF-8 names cannot be produced by
//!   a scan at all, so `../escape.rules` and `/etc/passwd` cannot match anything
//!   — not because they are filtered, but because nothing like them is ever in
//!   the set.
//!
//! # Infallible, like the parser
//!
//! [`discover`] returns a [`Discovery`] and never an error, for the same reason
//! [`super::RulesDoc::parse`] does: showing the user what is there is the whole
//! point of the screen, and one unreadable directory is not a reason to show
//! nothing. An unreadable directory, a symlink, a non-regular file, a too-large
//! file and a non-UTF-8 file each become a warning; the last two are still
//! **listed**, with `parsed: false`, because a file the user can see and cannot
//! edit is a better answer than a file that silently is not there.
//!
//! # Step 6: the CSV column preview
//!
//! A rules file addresses CSV columns by number (`%3`, `fields date, …`), so a
//! mapping screen that shows a bare `%3` is asking the user to go and open the
//! CSV themselves. [`Discovery::preview`] reads the first few rows of the data
//! file so the screen can say `Col 3  "GROCERY STORE"` instead.
//!
//! That means following a path *out of a file's contents*, which is a new kind
//! of reach for this module, so it is built to be refusable at every step:
//!
//! - **It cannot start anywhere else.** The only entry point takes a
//!   [`DiscoveredRules`] id, so the rules file is one this scan already admitted
//!   and the data file is resolved relative to *its* directory, inside the same
//!   root, tested with the same [`crate::parse::confine`].
//! - **A refusal is a value, not an error.** [`PreviewUnavailable`] names why,
//!   so the GUI can explain it, and *nothing is read* on any of those paths.
//! - **`source ... | CMD` is never run.** See [`PreviewUnavailable::SourceIsCommand`].
//! - **Every bound is on the read itself**, not on a trim afterwards:
//!   [`MAX_PREVIEW_BYTES`] through [`Read::take`], [`MAX_PREVIEW_ROWS`] through
//!   the record iterator, [`MAX_PREVIEW_COLUMNS`] and [`MAX_CELL_CHARS`] per
//!   record.
//! - **No path is disclosed.** [`Preview::data_label`] is a file *name*; no field
//!   and no reason carries a directory. Same rule, and same reason, as the
//!   warnings above.

use crate::edit::Fingerprint;
use crate::parse;
use crate::rules::{
    Item, ItemKind, OpaqueReason, RulesDoc, Separator, SourceSetting, Warning, sanitize_display,
};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

// ---------------------------------------------------------------------------
// Budgets
// ---------------------------------------------------------------------------

/// How far below the scan root the walk descends. The root itself is depth 0.
///
/// Real journal layouts nest `import/2026/` — two levels — and the deepest
/// plausible convention (`import/bank/checking/2026/statements/`) is five. Eight
/// is generous enough that no honest layout notices it and small enough that a
/// pathological tree cannot use depth alone to make the walk expensive.
const MAX_RULES_DEPTH: usize = 8;

/// How many rules files are RETURNED. One rules file per account is the norm, so
/// a household journal has a handful and a heavily automated one has dozens;
/// 200 is past any real corpus and still a list a GUI can render at once.
const MAX_RULES_FILES: usize = 200;

/// How many directory entries are EXAMINED — **the load-bearing bound**.
///
/// The other three caps bound the *answer*; this one bounds the *work*. A user
/// whose journal lives in `$HOME` (a completely ordinary choice) would otherwise
/// turn one scan into a full-disk walk, and no skip list can prevent that in
/// general: [`SKIP_DIRS`] names the directories that are commonly enormous, not
/// the ones that happen to be enormous here. Only counting entries and stopping
/// does. When it trips, [`Discovery::truncated`] is set, so the user is told the
/// list is a subset rather than shown a subset that looks complete.
const MAX_SCAN_ENTRIES: usize = 20_000;

/// The largest rules file that is read. Real ones are 2-8 KB; a megabyte is
/// three orders of magnitude past that, so anything larger is a mis-named file
/// (or a deliberate one), and reading it into memory to hash and parse buys
/// nothing. Over-size files are still listed — see [`DiscoveredRules::parsed`].
const MAX_RULES_BYTES: u64 = 1 << 20;

/// How many scan-level warnings are kept.
///
/// **Not in the step's spec**, and added for one reason: a hostile or merely
/// unlucky tree can produce a warning per skipped entry, up to
/// [`MAX_SCAN_ENTRIES`] of them, and a later HTTP layer puts these strings in a
/// dialog. Bounding the answer is the same discipline as bounding the walk. When
/// it trips, a final path-free warning says so.
const MAX_SCAN_WARNINGS: usize = 100;

/// Directory names never descended into. These are the ones that are routinely
/// enormous and never hold a user's import rules; skipping them is a
/// *performance* courtesy, not a security control — [`MAX_SCAN_ENTRIES`] is the
/// control. Every directory whose name starts with `.` is skipped too, which is
/// what keeps `.git` out.
const SKIP_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "vendor",
    "dist",
    "build",
    "__pycache__",
];

/// How much of a **data** file [`Discovery::preview`] reads. Bank CSVs are
/// routinely tens of megabytes and a year of transactions is nobody's column
/// header; 64 KiB is hundreds of records, which is three orders of magnitude
/// more than the handful actually shown.
///
/// Unlike [`MAX_RULES_BYTES`] this is **not** a refusal. A large CSV is the
/// normal case, not a suspicious one, so the read is capped and the preview says
/// [`Preview::truncated`] rather than declining to help.
const MAX_PREVIEW_BYTES: u64 = 64 * 1024;

/// How many sample records a preview returns. The screen shows "what is in
/// column 3", and three examples answer that — a fourth adds no information and
/// costs a row of vertical space next to the mapping controls.
const MAX_PREVIEW_ROWS: usize = 3;

/// How many columns of each record survive. hledger's own field vocabulary runs
/// out long before 64, and a record with more columns than this is a file the
/// mapping UI cannot usefully lay out anyway. It bounds the response against a
/// single 64 KiB line of commas, which is 32,768 columns.
const MAX_PREVIEW_COLUMNS: usize = 64;

/// How long one previewed cell may be, in `char`s. Wide enough for a real bank
/// description (which run to about 80), short enough that 64 of them cannot make
/// one response large. See [`sanitize_display`] for the rest of the treatment
/// these strings get before they reach a GUI.
const MAX_CELL_CHARS: usize = 120;

/// How many directory entries a `source` **glob** examines.
///
/// The same discipline as [`MAX_SCAN_ENTRIES`], at the scale this needs: a glob
/// is matched inside exactly one directory with no descent, so the only way it
/// can be expensive is a directory with a great many names. Far smaller than the
/// scan's budget because the answer here is a single file, not a listing.
const MAX_GLOB_CANDIDATES: usize = 2_000;

/// [`DiscoveredRules::revision`] for a file whose bytes were never read, so no
/// [`Fingerprint`] could be taken.
///
/// It is not a fingerprint token (those are always `LEN-HASH` in hex), so it can
/// never compare equal to one. A write path must gate on
/// [`DiscoveredRules::parsed`] rather than on this string: a file this module
/// declined to read is a file it will not claim to know the contents of.
const UNREAD_REVISION: &str = "unread";

// Why `parse.rs`'s MAX_INCLUDE_DEPTH / MAX_INCLUDE_FILES are NOT reused here,
// said out loud so the next reader does not "unify" the two sets:
//
// those bound recursion and fan-out through a graph the *journal author* wrote.
// Every edge is a deliberate directive, the work per edge is a full parse, and
// exceeding the budget is an error that refuses the journal outright. A
// directory walk is a different shape of work — the edges are whatever happens
// to be on disk, the per-entry cost is one `stat`, and exceeding the budget is
// not an error at all but a truncated list. Sharing one number would force one
// of the two to be wrong.

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// An absolute path that a [`Discovery`] produced.
///
/// The field is private and there is **no public constructor**, so "you may only
/// write to a file discovery returned" is enforced by the type system rather
/// than by code review. A later write path takes a `&RulesPath`, and the only
/// way to hold one is to have scanned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RulesPath(PathBuf);

impl RulesPath {
    /// The path, for the one caller that has to open the file.
    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

/// One `*.rules` file found in the journal's directory tree, plus the summary a
/// list view renders without opening it.
#[derive(Debug, Clone)]
pub struct DiscoveredRules {
    /// The relative path from the scan root, forward-slash separated. This IS
    /// the id — see the module docs on why resolution is string equality.
    pub id: String,
    /// Display name: the file name with a trailing `.csv.rules` / `.rules`
    /// stripped, so `checking.csv.rules` shows as `checking`. Display only; the
    /// id is what identifies the file.
    pub label: String,
    /// The file's size as of the scan's `stat`.
    pub size_bytes: u64,
    /// The file's mtime as of the scan's `stat` — **used ONLY to rank candidate
    /// rules files, and NEVER to detect change.**
    ///
    /// That distinction is the whole reason this comment exists. [`Fingerprint`]
    /// deliberately stopped recording an mtime (see `edit.rs` DL-3): every
    /// mtime-preserving copy tool breaks the inference from "timestamp unchanged"
    /// to "bytes unchanged", so a timestamp was removed rather than left lying
    /// around as a tempting shortcut. Nothing here reverses that. Change
    /// detection is [`DiscoveredRules::revision`], over the raw bytes, and it
    /// stays that way.
    ///
    /// Ranking is a different question with a different failure mode. A user has
    /// rules files going back years, several of which score identically against a
    /// dropped statement, and the one they touched most recently is the one they
    /// are still importing into — which naturally prefers the current year's
    /// without any filename ever being parsed for a year. Being wrong here costs
    /// a candidate list in a slightly worse order. Being wrong about change
    /// detection costs the user's edits. See
    /// [`crate::rules::matching::rank`], which is the only consumer.
    ///
    /// `None` on a platform or filesystem that does not report one.
    pub modified: Option<std::time::SystemTime>,
    /// [`Fingerprint::token`] over the file's **raw bytes** — the value a later
    /// write path uses as an `If-Match`, so a save cannot land on top of an edit
    /// made elsewhere. [`UNREAD_REVISION`] when the bytes were never read.
    pub revision: String,
    /// Whether the file was read, decoded as UTF-8 and parsed. `false` means the
    /// summary fields below are all empty and nothing here describes the
    /// contents — the entry exists so the file is visible, not so it is usable.
    pub parsed: bool,
    /// The file's top-level `account1`, from [`RulesDoc::settings`].
    pub account1: Option<String>,
    /// The file's top-level `account2`.
    pub account2: Option<String>,
    /// How many conditional constructs the file has: the editable ones plus the
    /// ones that stayed [`ItemKind::Opaque`] for a reason that names a
    /// conditional. This is the number a user recognizes as "my `if` rules".
    pub if_block_count: usize,
    /// How many of `if_block_count` classified as [`ItemKind::IfBlock`], and so
    /// are the ones a later step will let them edit one matcher at a time.
    pub editable_block_count: usize,
    /// How many items of any kind stayed [`ItemKind::Opaque`]. Opaque is never a
    /// failure (see the [`super`] module docs); this is how much of the file the
    /// editor will show read-only.
    pub opaque_item_count: usize,
    /// What discovery noticed about **this file** — over-size, unreadable, not
    /// UTF-8. Deliberately *not* the file's own parse warnings: those belong to
    /// the document a later per-file route returns, and duplicating them here
    /// would make a list response scale with the contents of every file in the
    /// tree. Names only the relative id.
    pub warnings: Vec<Warning>,
    /// The absolute path, unforgeable and inaccessible outside this module.
    path: RulesPath,
    /// `(st_dev, st_ino)` as of the scan, or `None` on a platform that has no
    /// such identity. See [`DiscoveredRules::identity_unchanged`].
    identity: Option<(u64, u64)>,
    /// What [`Discovery::preview`] needs the rules file to have *said*. Captured
    /// during the scan's parse rather than re-read on demand, so a preview opens
    /// exactly one file — the data file — and can never disagree with the
    /// summary above about which rules file it is describing.
    hints: DataHints,
}

/// The three directives a preview needs, lifted out of [`RulesDoc::settings`]
/// while the scan already has the document open.
///
/// Deliberately a snapshot rather than a borrow of the doc: keeping 200 parsed
/// rules files alive to answer one preview would trade a few hundred bytes for
/// a few megabytes.
#[derive(Debug, Clone, Default)]
struct DataHints {
    /// `source`, verbatim. Still never resolved, globbed or executed *here* —
    /// [`Discovery::locate`] is the only thing that interprets it, under guard.
    source: Option<SourceSetting>,
    /// `separator`, which beats the data file's extension when both are known.
    separator: Option<Separator>,
    /// `skip`, defaulted to hledger's own default of 0. This is what makes the
    /// record at index `skip - 1` the header.
    skip: u32,
}

/// Why a **new** rules file may not be created at an id.
///
/// One variant per guard in [`Discovery::resolve_new`], and the split matters to
/// a caller: two of them are about places this module declined to look and must
/// be reported indistinguishably, while the third is about the set a client can
/// already enumerate. See that method's docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CreateRefusal {
    /// Not a shape a scan could ever have produced, or one it would skip: a
    /// traversal, a hidden component, a `SKIP_DIRS` directory, a name not
    /// ending `.rules`.
    Malformed,
    /// Resolves outside the journal's own directory.
    OutsideRoot,
    /// The directory it would go in is not there, or is not a directory, or is
    /// a symlink. **No directory is ever created** — a rules file goes beside a
    /// journal that already exists.
    DirectoryMissing,
    /// Something is already at that name. Creating and editing are separate
    /// operations on purpose; this is where that separation is enforced.
    Exists,
}

/// The result of one scan: what was found, and whether it is all of it.
#[derive(Debug, Clone)]
pub struct Discovery {
    /// The canonical scan root. **Private and never leaves this module** — see
    /// the module docs on path disclosure, and [`Discovery::root_label`] for the
    /// one thing a GUI is given instead.
    root: PathBuf,
    /// The files found, sorted by [`DiscoveredRules::id`], so two scans of an
    /// unchanged tree produce byte-identical output.
    pub files: Vec<DiscoveredRules>,
    /// A cap was hit ([`MAX_SCAN_ENTRIES`], [`MAX_RULES_FILES`] or
    /// [`MAX_RULES_DEPTH`]) and the list is incomplete. Surfaced so the user is
    /// never silently shown a subset — a rules file that is simply *missing*
    /// from an imports screen is a bug report about the wrong thing.
    pub truncated: bool,
    /// What the walk skipped and why, each naming a **relative** path only.
    /// Bounded by [`MAX_SCAN_WARNINGS`]. Warnings about a file that was still
    /// listed live on that file instead.
    pub warnings: Vec<Warning>,
}

/// The first few rows of the data file a rules file describes, so a mapping
/// screen can label `%3` with what column 3 actually contains.
///
/// Everything here is **display only** and lossy: cells are sanitized and
/// truncated ([`sanitize_display`]), the row count is a sample, and nothing in
/// this type is ever written back anywhere. A `Preview` exists to put words next
/// to a column number, not to model the CSV.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    /// Whether there is anything below to show. `false` always comes with a
    /// `reason`, and means **nothing was read** — see [`PreviewUnavailable`].
    pub available: bool,
    /// Why the preview is unavailable, phrased as a variant rather than a string
    /// so the GUI writes the sentence and this module discloses no path.
    pub reason: Option<PreviewUnavailable>,
    /// The data file's NAME only — never a path. Display only, and `None` when
    /// no single concrete file was ever named (a refused `source`, or a glob
    /// that matched nothing).
    pub data_label: Option<String>,
    /// The delimiter the records below were split on: the rules file's
    /// `separator` if it has one, else inferred from the data file's extension.
    /// Reported because a mapping screen that shows the wrong columns is much
    /// easier to debug when it also says which character it split on.
    pub separator: char,
    /// The record at index `skip - 1`, when the rules file has `skip >= 1`.
    /// `None` otherwise — a file with no `skip` has no header row by definition,
    /// and inventing one would label every column with a data value.
    pub header: Option<Vec<String>>,
    /// Up to [`MAX_PREVIEW_ROWS`] records from immediately after `skip`.
    pub rows: Vec<Vec<String>>,
    /// The widest record seen, so the GUI can lay out a column per index even
    /// when the header is short. Never more than [`MAX_PREVIEW_COLUMNS`], since
    /// that is the width at which cells stop being kept at all.
    pub columns: usize,
    /// The read stopped at [`MAX_PREVIEW_BYTES`], so this is a preview of the
    /// data file's first bytes and not of the data file.
    ///
    /// Deliberately **not** set merely because `rows` is a sample: it always is,
    /// so a flag that was always true would tell a GUI nothing.
    pub truncated: bool,
}

/// Why a [`Preview`] has nothing to show. Each variant is a decision to refuse,
/// and on every one of them **nothing on disk was read**.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PreviewUnavailable {
    /// No data file was found: the rules file has no `source`, and the sibling
    /// its own name implies (`checking.csv.rules` -> `checking.csv`) is not
    /// there — or a `source` glob matched nothing.
    NoDataFile,
    /// The `source` directive contains a `|`, which makes it a **shell command
    /// hledger executes on import**. We never run it. Not with `Command`, not
    /// with a shell, not ever.
    ///
    /// That is the whole variant. There is no mode, flag or future step that
    /// turns it into an execution: a rules file is a document a user downloaded
    /// or was sent, so honouring the pipe would make "look at my import rules" a
    /// remote-code-execution primitive. `super::writable` closes the same hole on
    /// the write side, by refusing to let anyone *author* one through Ledgeline.
    SourceIsCommand,
    /// The `source` names a file outside the journal's own directory — by
    /// traversing out of it, by naming an absolute path elsewhere, by globbing
    /// somewhere above the final component, or by being a **bare filename**,
    /// which hledger resolves against `~/Downloads`. The whole imports feature is
    /// confined to the journal directory, and a data file is not an exception.
    ///
    /// Deliberately returned whether or not the named file exists, so the
    /// distinction between the two never reaches a dialog: a message that
    /// changes when a path outside the root happens to be there is a filesystem
    /// existence oracle.
    SourceOutsideRoot,
    /// The target is a symbolic link, a FIFO, a device, a socket or a directory.
    ///
    /// A symlink is refused rather than resolved, exactly as the scan refuses
    /// one. The FIFO is the case that actually bites: a `read` on one with no
    /// writer blocks forever, so it would not fail the request that asked for a
    /// preview — it would hang it.
    NotRegularFile,
    /// The target exists and is a regular file, but could not be opened or read.
    Unreadable,
    /// The target's bytes are not valid UTF-8. Ledgeline reads UTF-8 CSVs only;
    /// a rules file's `encoding` directive is not honoured here, and guessing at
    /// a code page would put mojibake next to a column number.
    NotUtf8,
    /// The target has no records at all, or none that survive the rules file's
    /// `skip`.
    Empty,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Find every `*.rules` file in the open journal's own directory tree.
///
/// `main_journal_file` is the journal's main source file — `Journal::source_files[0]`
/// (`model.rs`), which is canonicalized and always first. Its *directory* is the
/// scan root, via [`crate::parse::include_root_for`], so the rules surface and the
/// `include` guard confine to exactly the same place.
///
/// Infallible: an unreadable directory, a symlink, a too-large file and a
/// non-UTF-8 file each become a warning naming a RELATIVE path — never an error,
/// and never an absolute path.
///
/// A `main_journal_file` whose bytes are not valid UTF-8 is lossily converted for
/// the root computation, which then names a path that does not exist; the scan
/// fails closed on it (one warning, no files) rather than guessing at a
/// neighbouring directory.
#[must_use]
pub fn discover(main_journal_file: &Path) -> Discovery {
    Scan::new(parse::include_root_for(
        &main_journal_file.to_string_lossy(),
    ))
    .run()
}

impl Discovery {
    /// The **one** id -> path resolution in the codebase.
    ///
    /// Exact string equality against this scan's set. Deliberately not
    /// `root.join(id)`: joining would take a client-supplied string and turn it
    /// into a path, which is the operation every traversal bug is made of. Here
    /// the client's string is only ever *compared*, so the path it selects is
    /// one this module built from a `read_dir` name moments earlier.
    ///
    /// Callers should scan, resolve and write within one request. A stale id
    /// simply misses.
    #[must_use]
    pub fn resolve(&self, id: &str) -> Option<&DiscoveredRules> {
        self.files.iter().find(|file| file.id == id)
    }

    /// The scan root's final path component, for a GUI heading ("Rules in
    /// **ledger**"). **Not** the path: everything above the last component is
    /// exactly what must not be disclosed. A root with no final component (`/`,
    /// or a relative `.`) gets a neutral word instead.
    #[must_use]
    pub fn root_label(&self) -> String {
        self.root.file_name().map_or_else(
            || "journal".to_string(),
            |name| name.to_string_lossy().into_owned(),
        )
    }

    /// Where a **new** rules file called `id` would go — the one path in this
    /// crate built by joining a caller's string onto the root.
    ///
    /// # Why this exists at all, given [`resolve`](Self::resolve)
    ///
    /// [`resolve`](Self::resolve) can only ever return a file the scan already
    /// found, which is the whole of its security value and is exactly why it
    /// cannot serve a *create*: the file is not there yet, so there is nothing
    /// to match against. `root.join(id)` is unavoidable here, and this is the
    /// only function in either crate that performs it. It is `join`ed once,
    /// under every guard below, and the result is a [`RulesPath`] — so the write
    /// path downstream is unchanged and still cannot be handed a path from
    /// anywhere else.
    ///
    /// # The guards, in order, and what each is for
    ///
    /// 1. **Shape** ([`CreateRefusal::Malformed`]) — before any filesystem call,
    ///    and deliberately a *second* copy of the question `rules_api`'s own
    ///    `validate_id` asks. This module does not get to assume its caller
    ///    checked: a `..` reaching a `join` is the whole traversal bug class.
    /// 2. **Discoverability** ([`CreateRefusal::Malformed`]) — no component may
    ///    be hidden or in [`SKIP_DIRS`], because the scan skips those. Creating
    ///    a file the scan will never list would write something the user cannot
    ///    then open, which is a worse outcome than refusing.
    /// 3. **Confinement** ([`CreateRefusal::OutsideRoot`]) — the same
    ///    [`crate::parse::confine`] every other reach in this module uses, which
    ///    canonicalizes the deepest existing ancestor and re-appends the rest,
    ///    so a symlinked journal directory (`/tmp` → `/private/tmp` on macOS)
    ///    still compares equal.
    /// 4. **A real parent directory** ([`CreateRefusal::DirectoryMissing`]) —
    ///    `symlink_metadata`, so a symlinked directory is refused rather than
    ///    followed, exactly as the scan refuses one. No directory is ever
    ///    created: a rules file goes beside a journal that already exists.
    /// 5. **Nothing there already** ([`CreateRefusal::Exists`]) — of any file
    ///    type, symlinks included.
    ///
    /// Guard 5 is **not** what makes the create safe, and must not be relied on
    /// as if it were: it expires the moment it returns. The write itself has to
    /// be exclusive (`O_EXCL`), and `rules_api` performs it that way — this is
    /// the courtesy that produces a good error message, and the open is what
    /// produces the guarantee.
    ///
    /// # What each refusal may be told to a caller
    ///
    /// [`OutsideRoot`](CreateRefusal::OutsideRoot) and
    /// [`DirectoryMissing`](CreateRefusal::DirectoryMissing) are the two that
    /// could answer a question about the filesystem, so a caller must collapse
    /// them into the same sentence every other resolution failure returns.
    /// [`Exists`](CreateRefusal::Exists) is safe to report as itself: it is only
    /// reachable for a confined, non-hidden `*.rules` name below the root — the
    /// exact set `GET /api/rules` already publishes.
    ///
    /// # Errors
    /// [`CreateRefusal`], one variant per guard above.
    pub fn resolve_new(&self, id: &str) -> Result<RulesPath, CreateRefusal> {
        let parts: Vec<&str> = id.split('/').collect();
        let well_formed = !id.is_empty()
            && parts.len() <= MAX_RULES_DEPTH + 1
            && parts.iter().all(|part| {
                !part.is_empty()
                    && *part != "."
                    && *part != ".."
                    && !part.starts_with('.')
                    && !part.contains('\\')
                    && !part.contains(':')
                    && !part.chars().any(|c| c.is_ascii_control())
            })
            // Every directory component must be one the scan would descend
            // into, and the file name one it would list. See guard 2.
            && parts[..parts.len() - 1]
                .iter()
                .all(|part| !SKIP_DIRS.contains(part))
            && parts.last().is_some_and(|name| is_rules_name(name));
        if !well_formed {
            return Err(CreateRefusal::Malformed);
        }

        // THE join. Everything above is what earns it.
        let candidate = self.root.join(id);
        let Some(resolved) = parse::confine(&candidate, &self.root) else {
            return Err(CreateRefusal::OutsideRoot);
        };
        let Some(parent) = resolved.parent() else {
            return Err(CreateRefusal::DirectoryMissing);
        };
        // On the CANONICAL path's parent, and with `symlink_metadata`: the
        // parent of a canonical path exists if anything does, and asking about
        // the link rather than its target is what refuses a directory symlink.
        match std::fs::symlink_metadata(parent) {
            Ok(meta) if meta.file_type().is_dir() => {}
            _ => return Err(CreateRefusal::DirectoryMissing),
        }
        match std::fs::symlink_metadata(&resolved) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(RulesPath(resolved)),
            // Present, or unreadable in a way that is not "absent". Either way
            // this is not a name a create may have.
            _ => Err(CreateRefusal::Exists),
        }
    }

    /// Peek at the data file `id`'s rules describe, for column labels in the
    /// mapping UI. Read-only, size-capped, and confined to the scan root.
    ///
    /// `None` **only** when `id` does not resolve — see [`Discovery::resolve`],
    /// which is the same exact-string-equality lookup and the same reason a
    /// client-supplied string never becomes a path. Every other outcome is a
    /// [`Preview`]: a refusal is a [`PreviewUnavailable`] the GUI can explain,
    /// not an error and not an empty result that looks like a working one.
    ///
    /// The data file is found in one of two ways, in this order:
    ///
    /// 1. the rules file's **`source` directive**, resolved relative to the rules
    ///    file's own directory (which is what hledger does), and
    /// 2. otherwise the **sibling** its name implies: `checking.csv.rules` ->
    ///    `checking.csv`, beside it.
    ///
    /// A `source` containing a `|` is a shell command hledger runs on import.
    /// **We never run it. Not with `Command`, not with a shell, not ever** — see
    /// [`PreviewUnavailable::SourceIsCommand`].
    #[must_use]
    pub fn preview(&self, id: &str) -> Option<Preview> {
        let file = self.resolve(id)?;
        let hints = &file.hints;
        Some(match self.locate(file) {
            // No path was resolved, so there is no extension to infer from and
            // nothing whose name may be disclosed.
            Err(refusal) => Preview::unavailable(
                refusal.reason,
                refusal.label,
                delimiter_for(hints.separator.as_ref(), None),
            ),
            Ok(path) => read_preview(&path, hints),
        })
    }

    /// Resolve the data file `file` describes, applying every guard, and return
    /// the **canonical** path to read.
    ///
    /// The order of the two checks below is load-bearing and is not the order
    /// they read in. [`parse::confine`] runs *first*, before any `stat`, because
    /// its answer is the same whether or not a path outside the root exists —
    /// while "not found" versus "not a regular file" is a perfectly good oracle
    /// for whether `/etc/anything` is there. The symlink test then runs on the
    /// path **as constructed**, never on `confine`'s output: `confine`
    /// canonicalizes, and a canonical path has no symlinks left in it to refuse.
    fn locate(&self, file: &DiscoveredRules) -> Result<PathBuf, Refusal> {
        let Some(dir) = file.path.0.parent() else {
            // A rules file with no parent directory cannot be produced by a scan
            // (every id has at least one component below the root), so this is
            // unreachable rather than a case with a better answer.
            return Err(Refusal::bare(PreviewUnavailable::NoDataFile));
        };
        match &file.hints.source {
            Some(source) => self.locate_source(dir, source),
            None => {
                let Some(name) = sibling_data_name(&file.id) else {
                    return Err(Refusal::bare(PreviewUnavailable::NoDataFile));
                };
                self.admit(&dir.join(&name))
                    .map_err(|reason| Refusal::named(reason, name))
            }
        }
    }

    /// Interpret a `source` directive far enough to name one file, refusing
    /// every shape that would reach outside the journal's own directory.
    fn locate_source(&self, dir: &Path, source: &SourceSetting) -> Result<PathBuf, Refusal> {
        if source.has_command {
            return Err(Refusal::bare(PreviewUnavailable::SourceIsCommand));
        }
        // Trailing whitespace is never part of an intended filename, and a
        // directive's value runs verbatim to end of line (see the `super` module
        // docs on why that asymmetry is hledger's). Trimming here can only make
        // a preview appear where one would otherwise say "no data file"; it
        // cannot widen what is reachable, because every guard below still runs.
        let raw = source.raw.trim();
        if raw.is_empty() {
            return Err(Refusal::bare(PreviewUnavailable::NoDataFile));
        }
        // A `source` with no path separator at all is a BARE FILENAME, which
        // hledger resolves against `~/Downloads` — outside the journal directory
        // this feature is confined to, so it is refused rather than quietly
        // re-pointed at a sibling the user did not name.
        if !raw.contains(std::path::is_separator) {
            return Err(Refusal::bare(PreviewUnavailable::SourceOutsideRoot));
        }

        let value = Path::new(raw);
        let (Some(name), Some(parent)) = (
            value.file_name().and_then(std::ffi::OsStr::to_str),
            value.parent(),
        ) else {
            // A value that ends in `/` or `..`, or whose final component is not
            // UTF-8: it names a directory or nothing, never a data file.
            return Err(Refusal::bare(PreviewUnavailable::SourceOutsideRoot));
        };

        // A glob in anything but the final component would mean *walking* to
        // find the directory to look in, and a walk driven by a pattern out of a
        // file's contents is the thing this module exists not to do. Refused
        // outright — hledger allows it; we deliberately do not.
        if parent.to_string_lossy().contains(GLOB_CHARS) {
            return Err(Refusal::bare(PreviewUnavailable::SourceOutsideRoot));
        }

        // `join` on an absolute value yields the value, so an absolute `source`
        // is used as written and then refused by containment, not by shape.
        if name.contains(GLOB_CHARS) {
            self.newest_glob_match(&dir.join(parent), name)
        } else {
            self.admit(&dir.join(value))
                .map_err(|reason| Refusal::named(reason, name.to_string()))
        }
    }

    /// hledger reads the **newest** match of a `source` glob, so this does too.
    ///
    /// Matched with one `read_dir` of one directory — no descent, and no glob
    /// library — because the only patterns supported are `*` and `?` in the
    /// final component (see [`glob_matches`]). A candidate that fails a guard is
    /// skipped rather than refused, so the answer is the newest *eligible* file;
    /// the winner then goes through exactly the same [`Discovery::admit`] as a
    /// non-glob target.
    fn newest_glob_match(&self, dir: &Path, pattern: &str) -> Result<PathBuf, Refusal> {
        // Confine the DIRECTORY before listing it: a `read_dir` of somewhere
        // outside the root is already a disclosure, whatever is done with it.
        let Some(dir) = parse::confine(dir, &self.root) else {
            return Err(Refusal::bare(PreviewUnavailable::SourceOutsideRoot));
        };
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return Err(Refusal::bare(PreviewUnavailable::NoDataFile));
        };
        let pattern: Vec<char> = pattern.chars().collect();

        let best = entries
            .take(MAX_GLOB_CANDIDATES)
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name().into_string().ok()?;
                // A leading `.` is not matched by `*`, which is both hledger's
                // glob default and this module's own policy on dot entries. It
                // keeps a `.DS_Store` — very often the newest thing in a
                // download directory — from winning on mtime.
                if name.starts_with('.') || !glob_matches(&pattern, &name) {
                    return None;
                }
                let meta = std::fs::symlink_metadata(entry.path()).ok()?;
                if !meta.file_type().is_file() {
                    return None;
                }
                Some((meta.modified().ok()?, name))
            })
            // Newest wins; the name breaks a tie, so two files written in the
            // same clock tick still resolve the same way on every run.
            .max();

        let Some((_, name)) = best else {
            return Err(Refusal::bare(PreviewUnavailable::NoDataFile));
        };
        self.admit(&dir.join(&name))
            .map_err(|reason| Refusal::named(reason, name))
    }

    /// The guards every candidate data file passes, in the order argued for in
    /// [`Discovery::locate`].
    fn admit(&self, path: &Path) -> Result<PathBuf, PreviewUnavailable> {
        let Some(canonical) = parse::confine(path, &self.root) else {
            return Err(PreviewUnavailable::SourceOutsideRoot);
        };
        // `symlink_metadata`, and on the pre-canonical path: the point is to see
        // the link rather than what it points at.
        let meta = std::fs::symlink_metadata(path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                PreviewUnavailable::NoDataFile
            } else {
                PreviewUnavailable::Unreadable
            }
        })?;
        if !meta.file_type().is_file() {
            return Err(PreviewUnavailable::NotRegularFile);
        }
        Ok(canonical)
    }
}

/// A refusal plus the name it is about, when there is one to name.
///
/// The name is worth carrying because "checking.csv is not there" is a far more
/// actionable dialog than "no data file" — and a file *name* discloses nothing,
/// unlike the directory it sits in. Refusals that never got as far as one
/// concrete name carry `None`.
struct Refusal {
    reason: PreviewUnavailable,
    label: Option<String>,
}

impl Refusal {
    /// A refusal with no file name to show.
    fn bare(reason: PreviewUnavailable) -> Self {
        Self {
            reason,
            label: None,
        }
    }

    /// A refusal about one named file.
    ///
    /// [`PreviewUnavailable::SourceOutsideRoot`] is the exception, and it is
    /// enforced here rather than left to each call site: that refusal is about a
    /// place this module declined to look, so naming what it would have looked
    /// for makes the refusal describe a target OUTSIDE the root. Every other
    /// reason is about a file inside the journal's own directory, where a name is
    /// the useful half of the answer and discloses nothing.
    fn named(reason: PreviewUnavailable, label: String) -> Self {
        match reason {
            PreviewUnavailable::SourceOutsideRoot => Self::bare(reason),
            _ => Self {
                reason,
                label: Some(label),
            },
        }
    }
}

impl DiscoveredRules {
    /// The absolute path, for the one caller that has to open the file.
    #[must_use]
    pub fn path(&self) -> &RulesPath {
        &self.path
    }

    /// Re-`symlink_metadata` the target and require it is still a regular file
    /// with the same `(dev, ino)` recorded at scan time. **Called immediately
    /// before a write.**
    ///
    /// The scan proved the path was a regular file inside the root; that proof
    /// expires the moment the scan ends. Between the scan and the save, the name
    /// can be replaced with a symlink, a FIFO or a different file entirely, and
    /// a write that only re-checked *containment* would happily follow it. Inode
    /// identity is the check that says "the same file", not "a file with the
    /// same name".
    ///
    /// It does not say it *perfectly*: ext4 and tmpfs hand a just-freed inode
    /// number straight back to the next create, so a file removed and recreated
    /// under this name can present the very same `(dev, ino)`. That gap is why
    /// this is one half of a pair — the caller re-reads and re-fingerprints the
    /// bytes immediately before writing, and a recreated file either has
    /// different content, which that check refuses, or identical content, which
    /// is nothing to refuse.
    ///
    /// On a platform with no `(dev, ino)` — where `identity` is `None` — this
    /// degrades to the regular-file check alone, which is weaker, and honestly
    /// so: it can still refuse a name that became a link or a device, but not a
    /// name that became a different regular file.
    #[must_use]
    pub fn identity_unchanged(&self) -> bool {
        let Ok(meta) = std::fs::symlink_metadata(&self.path.0) else {
            return false;
        };
        meta.file_type().is_file() && self.identity == file_identity(&meta)
    }
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
struct Scan {
    root: PathBuf,
    files: Vec<DiscoveredRules>,
    warnings: Vec<Warning>,
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
    /// order.
    ///
    /// The order matters for more than tidiness: when a cap trips, *which*
    /// entries were examined decides which files survive, and `read_dir` order
    /// is filesystem- and even run-dependent. Sorting each directory makes a
    /// truncated result reproducible instead of arbitrary.
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
                if self.examined >= MAX_SCAN_ENTRIES || self.files.len() >= MAX_RULES_FILES {
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
        self.files.sort_by(|a, b| a.id.cmp(&b.id));
        Discovery {
            root: self.root,
            files: self.files,
            truncated: self.truncated,
            warnings: self.warnings,
        }
    }

    /// This directory's entries, sorted, and never more than the remaining entry
    /// budget.
    ///
    /// The `take` is not decoration: collecting a directory of a million names
    /// before checking the budget would let a single directory defeat
    /// [`MAX_SCAN_ENTRIES`] by allocating instead of by walking. One entry past
    /// the budget is taken so the caller's check still trips and sets
    /// `truncated`.
    ///
    /// A per-entry `read_dir` error (a name that vanished mid-walk) drops that
    /// entry silently; there is nothing to say about it that is both true and
    /// useful.
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
    /// guarantees; see the module docs.
    fn visit(&mut self, path: &Path, depth: usize, subdirs: &mut Vec<PathBuf>) {
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

        // `symlink_metadata`, never `metadata`: the whole point is to see the
        // link rather than what it points at.
        let Ok(meta) = std::fs::symlink_metadata(path) else {
            self.warn(format!("{id} could not be read and was skipped"));
            return;
        };
        let kind = meta.file_type();

        if kind.is_symlink() {
            self.warn(format!(
                "{id} is a symbolic link and was skipped; import rules are only read from real files inside the journal's own directory"
            ));
            return;
        }

        // A leading dot, on a directory OR on a file. A hidden entry is one the
        // user's own file browser does not show them, so offering `.hidden.rules`
        // for editing is offering something they cannot see — and a dot-file in a
        // journal directory is much more often a tool's leftover than a rules
        // file someone wants listed. For directories this is what keeps `.git/`
        // and `.direnv/` out. Silent, like [`SKIP_DIRS`]: a policy skip is not a
        // problem to report.
        //
        // Nothing legitimate is lost at the file end. A bare `.rules` is already
        // refused by [`is_rules_name`] — it would strip to an empty label — so
        // this only removes names the user deliberately hid. [`Discovery::preview`]
        // has held a dot entry to the same rule from the start; this makes the
        // scan agree with it.
        if name.starts_with('.') {
            return;
        }

        if kind.is_dir() {
            if SKIP_DIRS.contains(&name) {
                return;
            }
            if depth + 1 > MAX_RULES_DEPTH {
                self.truncated = true;
                return;
            }
            subdirs.push(path.to_path_buf());
            return;
        }

        let named_rules = is_rules_name(name);
        if !kind.is_file() {
            // FIFOs, devices, sockets. A FIFO is the one that actually bites: a
            // `read` on one with no writer blocks forever, which would hang the
            // request that asked for the list, not merely fail it.
            if named_rules {
                self.warn(format!("{id} is not a regular file and was skipped"));
            }
            return;
        }
        if !named_rules {
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

        let file = describe(id, resolved, &meta);
        self.files.push(file);
    }

    /// Record a scan-level warning, up to [`MAX_SCAN_WARNINGS`] of them plus one
    /// path-free note saying there were more.
    fn warn(&mut self, message: String) {
        if self.warnings.len() < MAX_SCAN_WARNINGS {
            self.warnings.push(scan_warning(message));
        } else if self.warnings.len() == MAX_SCAN_WARNINGS {
            self.warnings.push(scan_warning(format!(
                "more than {MAX_SCAN_WARNINGS} entries were skipped; only the first {MAX_SCAN_WARNINGS} are listed"
            )));
        }
    }
}

// ---------------------------------------------------------------------------
// One file
// ---------------------------------------------------------------------------

/// Read, fingerprint, parse and summarize one admitted file.
///
/// `meta` is the scan's `symlink_metadata`, so `size_bytes` and the recorded
/// identity describe the same `stat` that admitted the file.
fn describe(id: String, path: PathBuf, meta: &std::fs::Metadata) -> DiscoveredRules {
    let mut warnings = Vec::new();
    let mut file = DiscoveredRules {
        label: label_for(&id),
        size_bytes: meta.len(),
        // Ranking only. See the field.
        modified: meta.modified().ok(),
        revision: UNREAD_REVISION.to_string(),
        parsed: false,
        account1: None,
        account2: None,
        if_block_count: 0,
        editable_block_count: 0,
        opaque_item_count: 0,
        warnings: Vec::new(),
        path: RulesPath(path),
        identity: file_identity(meta),
        hints: DataHints::default(),
        id,
    };

    match read_capped(&file.path.0) {
        Err(ReadStop::TooLarge) => warnings.push(file_warning(format!(
            "{} is larger than {} KiB, so it is listed but not read",
            file.id,
            MAX_RULES_BYTES / 1024
        ))),
        Err(ReadStop::Unreadable) => warnings.push(file_warning(format!(
            "{} could not be read, so it is listed but its contents are unavailable",
            file.id
        ))),
        Ok(bytes) => {
            // The fingerprint is over the RAW bytes, so it covers a file this
            // module could not decode just as well as one it could.
            file.revision = Fingerprint::of_bytes(&bytes).token();
            match String::from_utf8(bytes) {
                Ok(text) => {
                    summarize(&mut file, &RulesDoc::parse(&text));
                    file.parsed = true;
                }
                Err(_) => warnings.push(file_warning(format!(
                    "{} is not valid UTF-8, so it is listed but not parsed; Ledgeline reads and \
                     writes UTF-8 rules files only",
                    file.id
                ))),
            }
        }
    }

    file.warnings = warnings;
    file
}

/// Why a file's bytes were not obtained.
enum ReadStop {
    /// Past [`MAX_RULES_BYTES`], or it grew past it mid-read.
    TooLarge,
    /// Open or read failed, or the name stopped being a regular file between the
    /// scan's `stat` and the open.
    Unreadable,
}

/// Read a file, refusing anything past [`MAX_RULES_BYTES`].
///
/// The size is re-checked against the **open handle's** metadata rather than the
/// scan's, so the decision to read cannot be made about one file and carried out
/// on another. The `take` then bounds the read itself, because a file that grows
/// between the two would otherwise be read in full.
fn read_capped(path: &Path) -> Result<Vec<u8>, ReadStop> {
    let file = std::fs::File::open(path).map_err(|_| ReadStop::Unreadable)?;
    let meta = file.metadata().map_err(|_| ReadStop::Unreadable)?;
    if !meta.file_type().is_file() {
        return Err(ReadStop::Unreadable);
    }
    if meta.len() > MAX_RULES_BYTES {
        return Err(ReadStop::TooLarge);
    }
    let mut bytes = Vec::new();
    file.take(MAX_RULES_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ReadStop::Unreadable)?;
    if bytes.len() as u64 > MAX_RULES_BYTES {
        return Err(ReadStop::TooLarge);
    }
    Ok(bytes)
}

/// Fill in the summary a list view renders without opening the file.
fn summarize(file: &mut DiscoveredRules, doc: &RulesDoc) {
    let settings = doc.settings();
    file.hints = DataHints {
        source: settings.source.map(|setting| setting.value),
        separator: settings.separator.map(|setting| setting.value),
        // hledger's default is 0. `Settings::skip` is `None` for "the file does
        // not say", which is not the same thing, so the default is chosen here
        // rather than pretended to have been read.
        skip: settings.skip.map_or(0, |setting| setting.value),
    };
    file.account1 = settings.account1.map(|setting| setting.value);
    file.account2 = settings.account2.map(|setting| setting.value);
    file.editable_block_count = doc
        .items()
        .iter()
        .filter(|item| matches!(item.kind, ItemKind::IfBlock(_)))
        .count();
    file.opaque_item_count = doc
        .items()
        .iter()
        .filter(|item| matches!(item.kind, ItemKind::Opaque(_)))
        .count();
    let opaque_conditionals = doc
        .items()
        .iter()
        .filter_map(Item::opaque)
        .filter(|opaque| is_conditional(opaque.reason))
        .count();
    file.if_block_count = file.editable_block_count + opaque_conditionals;
}

/// Whether an [`OpaqueReason`] can only have come from a conditional construct.
///
/// [`OpaqueReason::Unclassified`] is deliberately **not** here even though a
/// degenerate `if` (no matcher, or no assignment) lands there: so does every
/// line that matches no rule shape at all, and inflating a user-facing count by
/// guessing which is which is worse than under-counting a file hledger would
/// reject anyway.
fn is_conditional(reason: OpaqueReason) -> bool {
    match reason {
        OpaqueReason::IfTable
        | OpaqueReason::CombinedMatcher
        | OpaqueReason::MatchGroup
        | OpaqueReason::CommentLikeMatcher
        | OpaqueReason::ControlFlowInBlock
        | OpaqueReason::UnparsedBlockBody => true,
        OpaqueReason::UnparsedDirective | OpaqueReason::Unclassified => false,
    }
}

// ---------------------------------------------------------------------------
// One data file — the CSV column preview
// ---------------------------------------------------------------------------

impl Preview {
    /// A refusal, carrying the reason and nothing that was not already known
    /// before any read was attempted.
    fn unavailable(
        reason: PreviewUnavailable,
        data_label: Option<String>,
        separator: char,
    ) -> Self {
        Self {
            available: false,
            reason: Some(reason),
            data_label,
            separator,
            header: None,
            rows: Vec::new(),
            columns: 0,
            truncated: false,
        }
    }
}

/// Read `path` and shape its first records into a [`Preview`].
///
/// Called only on a path [`Discovery::admit`] already cleared, so this is the
/// bounded read and nothing else: the containment, symlink and file-type
/// arguments are all made before control gets here.
fn read_preview(path: &Path, hints: &DataHints) -> Preview {
    let label = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .map(|name| sanitize_display(name, MAX_CELL_CHARS));
    let separator = delimiter_for(hints.separator.as_ref(), Some(path));

    let (text, truncated) = match read_preview_bytes(path) {
        Ok(read) => read,
        Err(reason) => return Preview::unavailable(reason, label, separator),
    };

    // `flexible` is not a convenience: a bank CSV whose trailer row is short (or
    // whose quoting is a little wrong) must PREVIEW, not fail — the user came
    // here to be shown what is in the file, including the mess.
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter_byte(separator))
        .flexible(true)
        .has_headers(false)
        .from_reader(text.as_bytes());

    let skip = usize::try_from(hints.skip).unwrap_or(usize::MAX);
    let records: Vec<Vec<String>> = reader
        .records()
        // A malformed record ends the preview rather than being skipped past:
        // once the reader has lost the plot, later records are not what the file
        // says, and showing them beside a column number would be a lie.
        .map_while(Result::ok)
        .take(skip.saturating_add(MAX_PREVIEW_ROWS))
        .map(|record| {
            record
                .iter()
                .take(MAX_PREVIEW_COLUMNS)
                .map(|cell| sanitize_display(cell, MAX_CELL_CHARS))
                .collect()
        })
        .collect();

    // hledger skips the first `skip` records, so the LAST of them is the header
    // if there is one at all. `skip 0` means the file has no header row, and
    // labelling the columns with record 0's values would present data as names.
    let header = skip.checked_sub(1).and_then(|at| records.get(at)).cloned();
    let rows: Vec<Vec<String>> = records.into_iter().skip(skip).collect();
    let columns = header
        .iter()
        .chain(rows.iter())
        .map(Vec::len)
        .max()
        .unwrap_or(0);

    if header.is_none() && rows.is_empty() {
        return Preview::unavailable(PreviewUnavailable::Empty, label, separator);
    }
    Preview {
        available: true,
        reason: None,
        data_label: label,
        separator,
        header,
        rows,
        columns,
        truncated,
    }
}

/// Read at most [`MAX_PREVIEW_BYTES`] of `path`, decoded as UTF-8, plus whether
/// the read stopped at the cap.
///
/// Two details are the whole function:
///
/// - The cap is applied by [`Read::take`], so a 40 GB CSV is never in memory —
///   reading and then truncating would be a cap on the *answer*, not on the work.
/// - When the cap is hit, the trailing **partial line is dropped**, at the last
///   `\n` in the bytes. That is what stops a multi-byte character sliced in half
///   at 65,536 bytes from reporting a perfectly good UTF-8 file as
///   [`PreviewUnavailable::NotUtf8`], and it is independently the right thing:
///   the record that line belongs to is truncated, and half a record shown
///   beside a column number is worse than no record.
fn read_preview_bytes(path: &Path) -> Result<(String, bool), PreviewUnavailable> {
    let file = std::fs::File::open(path).map_err(|_| PreviewUnavailable::Unreadable)?;
    // Re-asked of the OPEN HANDLE, so the decision to read cannot be made about
    // one file and carried out on another — the same discipline as
    // [`read_capped`], and here it is also what refuses a name that became a
    // FIFO between `admit` and this open.
    let meta = file
        .metadata()
        .map_err(|_| PreviewUnavailable::Unreadable)?;
    if !meta.file_type().is_file() {
        return Err(PreviewUnavailable::NotRegularFile);
    }

    let mut bytes = Vec::new();
    file.take(MAX_PREVIEW_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|_| PreviewUnavailable::Unreadable)?;

    let truncated = u64::try_from(bytes.len()).unwrap_or(u64::MAX) >= MAX_PREVIEW_BYTES;
    if truncated {
        match bytes.iter().rposition(|byte| *byte == b'\n') {
            Some(at) => bytes.truncate(at + 1),
            // 64 KiB with no line break at all: there is no complete record in
            // it, so there is nothing to keep.
            None => bytes.clear(),
        }
    }
    String::from_utf8(bytes)
        .map(|text| (text, truncated))
        .map_err(|_| PreviewUnavailable::NotUtf8)
}

/// The delimiter a data file's records are split on.
///
/// The rules file's `separator` **beats the extension** — it is what hledger
/// itself obeys, and it is the user saying so explicitly about this very file,
/// where the extension is only a convention about how the file was named.
///
/// A `separator` that is not a single ASCII byte falls back to the extension:
/// hledger's own reader takes one byte too, so honouring a multi-byte character
/// would be splitting records on something hledger cannot split on.
fn delimiter_for(hint: Option<&Separator>, path: Option<&Path>) -> char {
    hint.map(separator_char)
        .filter(char::is_ascii)
        .unwrap_or_else(|| path.map_or(',', delimiter_for_extension))
}

/// What a [`Separator`] means as a character. `tab`/`space` are words in the
/// file and characters everywhere else.
fn separator_char(separator: &Separator) -> char {
    match separator {
        Separator::Char(character) => *character,
        Separator::Tab { .. } => '\t',
        Separator::Space { .. } => ' ',
    }
}

/// The delimiter a data file's **extension** implies, matching hledger's own
/// mapping. Anything else — including no extension at all — is a comma, which is
/// what an unlabelled data file overwhelmingly is.
fn delimiter_for_extension(path: &Path) -> char {
    match path
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("ssv") => ';',
        Some("tsv") => '\t',
        _ => ',',
    }
}

/// The delimiter as the single byte `csv::ReaderBuilder` takes.
///
/// [`delimiter_for`] only ever yields an ASCII character, so the fallback is
/// unreachable; it is spelled out rather than asserted because a panic in a
/// preview would take down a request that was only ever asked to look.
fn delimiter_byte(delimiter: char) -> u8 {
    u8::try_from(u32::from(delimiter)).unwrap_or(b',')
}

/// The data file `id`'s own name implies: the file name with the trailing
/// `.rules` removed, so `checking.csv.rules` -> `checking.csv`.
///
/// Only `.rules` comes off, never `.csv.rules` as [`label_for`] strips for
/// display — the `.csv` is the data file's extension, and it is what
/// [`delimiter_for_extension`] reads.
///
/// The slice goes through `str::get` for the same reason [`is_rules_name`] does:
/// the sixth-from-last *byte* of a name need not be a `char` boundary.
fn sibling_data_name(id: &str) -> Option<String> {
    const SUFFIX: &str = ".rules";
    let name = id.rsplit('/').next().unwrap_or(id);
    name.get(..name.len().checked_sub(SUFFIX.len())?)
        .filter(|stem| !stem.is_empty())
        .map(str::to_string)
}

/// The two glob metacharacters supported in a `source`. `[a-z]` classes and `**`
/// are deliberately absent: they are not needed to name a bank's dated export,
/// and every pattern feature is a way for a file's contents to steer a search.
const GLOB_CHARS: [char; 2] = ['*', '?'];

/// Whether `name` matches `pattern`, where `*` matches any run of characters and
/// `?` matches exactly one.
///
/// Iterative with one backtrack point, never recursive — the same reason
/// [`Scan`] uses an explicit stack. The obvious recursive matcher recurses once
/// per `*`, and a pattern is a string out of a *file's contents*, so its depth
/// is chosen by whoever wrote the rules file. A stack overflow is a `SIGABRT`
/// and not a catchable panic.
///
/// Comparison is by `char` and case-sensitive, matching hledger's globbing on
/// the platforms this runs on. A `char` at a time (not a byte) so a `?` matches
/// one character and never half of one.
fn glob_matches(pattern: &[char], name: &str) -> bool {
    let name: Vec<char> = name.chars().collect();
    let mut at_pattern = 0usize;
    let mut at_name = 0usize;
    // Where to resume if the current `*` turns out to have matched too little.
    let mut star: Option<(usize, usize)> = None;

    while at_name < name.len() {
        match pattern.get(at_pattern) {
            Some('*') => {
                star = Some((at_pattern, at_name));
                at_pattern += 1;
            }
            Some('?') => {
                at_pattern += 1;
                at_name += 1;
            }
            Some(literal) if *literal == name[at_name] => {
                at_pattern += 1;
                at_name += 1;
            }
            // A mismatch is only fatal if there is no `*` to give more to.
            _ => match star {
                Some((star_at, matched_to)) => {
                    at_pattern = star_at + 1;
                    at_name = matched_to + 1;
                    star = Some((star_at, matched_to + 1));
                }
                None => return false,
            },
        }
    }
    // Trailing `*`s may still match nothing; anything else is a shortfall.
    pattern[at_pattern.min(pattern.len())..]
        .iter()
        .all(|remaining| *remaining == '*')
}

// ---------------------------------------------------------------------------
// Names, ids and warnings
// ---------------------------------------------------------------------------

/// Whether `name` names a rules file: it ends in `.rules`, ASCII-case-insensitively,
/// and is more than just `.rules` (which is a dotfile, and would strip to an
/// empty label).
///
/// `str::get` rather than `name[..]`: the last six *bytes* of a name are not
/// necessarily a `char` boundary — `a€bcde` is eight bytes whose sixth-from-last
/// lands mid-`€` — and indexing there is a panic. A filename is attacker-chosen
/// input on this path, so this is a real one, not a theoretical one.
fn is_rules_name(name: &str) -> bool {
    const SUFFIX: &str = ".rules";
    name.len() > SUFFIX.len()
        && name
            .get(name.len() - SUFFIX.len()..)
            .is_some_and(|tail| tail.eq_ignore_ascii_case(SUFFIX))
}

/// The display name: the file name with a trailing `.csv.rules` or `.rules`
/// stripped, so `import/2026/bank.csv.rules` shows as `bank`.
///
/// `.csv.rules` is tried first because stripping only `.rules` would leave
/// `bank.csv`, which reads as the data file rather than the rules for it.
///
/// The slice goes through `str::get` for the same reason [`is_rules_name`] does.
///
/// `pub` for the create path, which has to label a file no scan has produced
/// yet. Sharing this rather than re-deriving it there is what stops a drafted
/// document being titled differently from the same file once it is on disk.
#[must_use]
pub fn label_for(id: &str) -> String {
    let name = id.rsplit('/').next().unwrap_or(id);
    let lower = name.to_ascii_lowercase();
    for suffix in [".csv.rules", ".rules"] {
        if lower.ends_with(suffix)
            && let Some(stem) = name.get(..name.len() - suffix.len())
        {
            return stem.to_string();
        }
    }
    name.to_string()
}

/// `path` relative to `root`, forward-slash separated, or `None` if it is not
/// below `root` or has any component that is not a plain UTF-8 name.
///
/// Requiring every component to be [`Component::Normal`] is a guard, not
/// tidiness: it is what makes it impossible for a `.`, a `..`, a root or a
/// Windows prefix to ever appear inside an id, and therefore impossible for a
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

/// A warning about the scan rather than about a line of a file.
///
/// [`Warning::item`] is `None` and [`Warning::line`] is 0: there is no item and
/// no line, and 0 is not a line number a parser can produce (they are 1-based).
fn scan_warning(message: String) -> Warning {
    Warning {
        item: None,
        line: 0,
        message,
    }
}

/// A warning about one discovered file as a whole. Same shape as
/// [`scan_warning`]; named separately because the two land in different lists.
fn file_warning(message: String) -> Warning {
    scan_warning(message)
}

/// `(st_dev, st_ino)` — the pair that says "the same file", not "a file with the
/// same name". See [`DiscoveredRules::identity_unchanged`].
#[cfg(unix)]
fn file_identity(meta: &std::fs::Metadata) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    Some((meta.dev(), meta.ino()))
}

/// No portable inode identity outside Unix. Recorded as `None`, which
/// [`DiscoveredRules::identity_unchanged`] treats as "check what can be checked"
/// — the regular-file test alone — rather than as a failure that would make the
/// feature unusable there.
#[cfg(not(unix))]
fn file_identity(_meta: &std::fs::Metadata) -> Option<(u64, u64)> {
    None
}

// ---------------------------------------------------------------------------
// Unit tests — the pure helpers. The guards are tested end-to-end against a real
// filesystem in `tests/rules_security.rs`, which is where they can actually be
// exercised.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_names_are_matched_case_insensitively_and_never_bare() {
        assert!(is_rules_name("checking.csv.rules"));
        assert!(is_rules_name("a.RULES"));
        assert!(is_rules_name("x.Rules"));
        assert!(
            !is_rules_name(".rules"),
            "a bare dotfile is not a rules file"
        );
        assert!(!is_rules_name("rules"));
        assert!(!is_rules_name("checking.rules.bak"));
        assert!(!is_rules_name(""));
    }

    #[test]
    fn a_multibyte_name_shorter_than_the_suffix_check_does_not_panic() {
        // `a€bcde` is eight bytes whose sixth-from-last is inside the `€`.
        // Indexing there would panic, and a filename is attacker-chosen input.
        for name in ["a€bcde", "€", "€€", "€.rules", "a€.rules", "日本語のルール"]
        {
            let _ = is_rules_name(name);
            let _ = label_for(name);
        }
        assert!(is_rules_name("a€.rules"));
        assert_eq!(label_for("a€.rules"), "a€");
        assert!(!is_rules_name("a€bcde"));
    }

    #[test]
    fn labels_strip_the_longest_suffix_first() {
        assert_eq!(label_for("import/2026/bank.csv.rules"), "bank");
        assert_eq!(label_for("checking.rules"), "checking");
        assert_eq!(label_for("a.CSV.RULES"), "a");
        assert_eq!(label_for("odd.name.rules"), "odd.name");
    }

    #[test]
    fn ids_are_relative_forward_slash_paths_of_plain_components() {
        let root = Path::new("/j");
        assert_eq!(
            relative_id(root, Path::new("/j/import/2026/b.rules")).as_deref(),
            Some("import/2026/b.rules")
        );
        assert_eq!(relative_id(root, Path::new("/j")), None, "the root itself");
        assert_eq!(relative_id(root, Path::new("/other/b.rules")), None);
    }

    #[test]
    fn a_scan_warning_points_at_no_line() {
        let warning = scan_warning("x".to_string());
        assert_eq!(warning.item, None);
        assert_eq!(warning.line, 0);
    }

    // -----------------------------------------------------------------------
    // Step 6 — the preview's pure helpers
    // -----------------------------------------------------------------------

    #[test]
    fn the_sibling_data_name_strips_only_the_rules_suffix() {
        // `label_for` strips `.csv.rules` as a unit for DISPLAY; this must not,
        // because the `.csv` is the data file's extension and is what
        // `delimiter_for_extension` reads.
        assert_eq!(
            sibling_data_name("checking.csv.rules").as_deref(),
            Some("checking.csv")
        );
        assert_eq!(
            sibling_data_name("import/2026/bank.csv.rules").as_deref(),
            Some("bank.csv")
        );
        assert_eq!(sibling_data_name("plain.rules").as_deref(), Some("plain"));
        assert_eq!(
            sibling_data_name("a€.rules").as_deref(),
            Some("a€"),
            "a multi-byte name must not be sliced off a char boundary"
        );
        // Names a scan cannot produce, checked anyway: the helper must answer
        // rather than panic, whatever it is handed.
        assert_eq!(sibling_data_name(".rules"), None, "no stem left");
        assert_eq!(sibling_data_name("short"), None);
        assert_eq!(sibling_data_name(""), None);
    }

    #[test]
    fn the_extension_maps_to_hledgers_own_delimiters() {
        let delimiter = |name: &str| delimiter_for_extension(Path::new(name));
        assert_eq!(delimiter("a.csv"), ',');
        assert_eq!(delimiter("a.ssv"), ';');
        assert_eq!(delimiter("a.tsv"), '\t');
        assert_eq!(delimiter("a.TSV"), '\t', "matched case-insensitively");
        assert_eq!(delimiter("a.txt"), ',', "anything else is a comma");
        assert_eq!(delimiter("bank"), ',', "and so is no extension at all");
    }

    #[test]
    fn the_separator_directive_beats_the_extension_unless_it_is_not_one_byte() {
        let tsv = Path::new("a.tsv");
        assert_eq!(delimiter_for(None, Some(tsv)), '\t', "no directive");
        assert_eq!(
            delimiter_for(Some(&Separator::Char(';')), Some(tsv)),
            ';',
            "the directive wins"
        );
        assert_eq!(
            delimiter_for(
                Some(&Separator::Space {
                    raw: "SPACE".into()
                }),
                Some(tsv)
            ),
            ' '
        );
        assert_eq!(
            delimiter_for(Some(&Separator::Char('€')), Some(tsv)),
            '\t',
            "a multi-byte separator is not one hledger's reader can split on \
             either, so the extension is the better answer"
        );
        assert_eq!(delimiter_for(None, None), ',', "nothing known at all");
    }

    #[test]
    fn the_delimiter_byte_never_panics_on_a_non_ascii_char() {
        // Unreachable via `delimiter_for`, which filters to ASCII — asserted so
        // a future caller cannot make a preview panic instead of degrade.
        assert_eq!(delimiter_byte(','), b',');
        assert_eq!(delimiter_byte('\t'), b'\t');
        assert_eq!(delimiter_byte('€'), b',');
    }

    #[test]
    fn globs_match_only_star_and_question_in_one_component() {
        let matches =
            |pattern: &str, name: &str| glob_matches(&pattern.chars().collect::<Vec<_>>(), name);
        assert!(matches("bank*.csv", "bank.csv"), "`*` may match nothing");
        assert!(matches("bank*.csv", "bank-2026-07.csv"));
        assert!(!matches("bank*.csv", "statement.csv"));
        assert!(
            !matches("bank*.csv", "bank.csv.bak"),
            "anchored at both ends"
        );
        assert!(matches("bank-?.csv", "bank-1.csv"));
        assert!(!matches("bank-?.csv", "bank-12.csv"), "`?` is exactly one");
        assert!(matches("*", "anything"));
        assert!(matches("*", ""), "and `*` alone matches the empty name");
        assert!(matches("**", "abc"), "adjacent stars are not special");
        assert!(matches("a*b*c", "abc"));
        assert!(matches("a*b*c", "axxbyyc"));
        assert!(!matches("a*b*c", "axxbyy"));
        // A `?` counts CHARACTERS, not bytes: `é` is two bytes and one `?`.
        assert!(matches("caf?.csv", "café.csv"));
        assert!(!matches("caf??.csv", "café.csv"));
        // A pattern is a string out of a file's contents, so pathological ones
        // must terminate quickly rather than recurse. `a*a*a*…*b` over a run of
        // `a`s with no `b` is the classic case that makes a backtracking matcher
        // exponential; one backtrack point keeps it linear in the two lengths.
        assert!(!matches(&format!("{}b", "*a".repeat(40)), &"a".repeat(60)));
        assert!(matches(&"*a".repeat(40), &"a".repeat(60)));
        assert!(!matches(&"*a".repeat(40), &"a".repeat(30)));
    }
}
