//! The HTTP surface for CSV **import rules** files (Imports, steps 7-8): list
//! what is there, read one, preview the data file it describes, and save it
//! back.
//!
//! Steps 2-6 built the engine — a format-preserving span-tiling model
//! ([`ledgeline_core::rules`]), a classifier, renderers that splice one leaf and
//! leave a column-aligned file aligned, a discovery scan that decides which
//! files this feature may ever touch, and a CSV preview. This module is the
//! wiring, and it is deliberately thin: every decision that could damage a
//! user's file was made in the engine, where it is unit-testable and where the
//! type system enforces it.
//!
//! - `GET    /api/rules`                  — the discovery listing.
//! - `GET    /api/rules/{*id}`            — one parsed document.
//! - `GET    /api/rules-preview/{*id}`    — the first few rows of its data file.
//! - `PUT    /api/rules/{*id}`            — save a whole document.
//!
//! # Why `preview` is on its own prefix
//!
//! The obvious spelling is `/api/rules/{*id}/preview`, and axum 0.8 refuses it:
//! a `{*wildcard}` is greedy and its matcher rejects the registration outright
//! (*"Insertion failed due to conflict with previously registered route"*).
//! Fighting that would mean giving up catch-all ids, and an id genuinely
//! contains slashes (`import/2026/bank.csv.rules`). A sibling prefix costs one
//! line of routing and keeps the id semantics **identical** on both routes: the
//! same string, through the same [`validate_id`], resolved by the same
//! [`Discovery::resolve`].
//!
//! # Five independent security layers
//!
//! Each is load-bearing on its own; none is allowed to lean on another.
//!
//! 1. **[`validate_id`] — syntactic, before ANY filesystem call.** This is why
//!    the 400-vs-404 split is decided on *shape* rather than on existence: a
//!    route that answered differently for `../../etc/passwd.rules` depending on
//!    what is on disk would be an existence oracle.
//! 2. **Discovery-set membership.** [`Discovery::resolve`] is the only id→path
//!    resolution in the codebase, by exact string equality against a set scanned
//!    *in this request*. `root.join(id)` appears nowhere in this crate, and
//!    [`RulesPath`](ledgeline_core::rules::RulesPath) has no public constructor,
//!    so that is enforced by the type system rather than by review.
//! 3. **Confinement, file type and symlink refusal** already live inside
//!    [`rules::discover`]. Nothing here duplicates or weakens them.
//! 4. **Content provenance.** Every byte written is either a byte read from that
//!    file moments earlier or renderer output over validated typed fields —
//!    guaranteed structurally, because [`ItemBody`] has no raw-text variant and
//!    no [`WireItemIn`] variant carries one either. `source`, `archive` and
//!    `include` are keep-only in the engine; that refusal surfaces here as a
//!    `400`.
//! 5. **No resolved path is ever echoed.** Errors quote only the caller's own
//!    id, clipped and escaped ([`quoted`]). Every resolution failure returns the
//!    same sentence, so the route cannot be used to probe the filesystem.
//!
//! # Caching: `no-store`, and no `ETag`
//!
//! All three `GET`s answer `Cache-Control: no-store` and carry no `ETag`, unlike
//! every other read endpoint. The `ETag` is one per-journal generation counter
//! shared by all of them ([`crate::next_etag`]), so there is no third option:
//! bumping it for a rules-file change would invalidate the SPA's cached
//! `/transactions` body — hundreds of megabytes at 200k transactions — for a
//! change that affects no transaction, and *not* bumping it while serving from
//! the snapshot would hand out a stale document under a fresh tag.
//!
//! Rules files also stay **out** of [`AppState::source_files`], the snapshot and
//! the file watcher, and that is the same decision from the other side.
//! `source_files()` is contractually "the files this journal was parsed from",
//! and a `.rules` change invalidates no transaction — so routing one through
//! `reload_journal` would buy a full reparse and republish for nothing, which is
//! a PERF-4 regression by construction. A rules document is cheap to re-fetch
//! (kilobytes, one directory walk) and it is always re-read for a save anyway.
//!
//! # Optimistic concurrency
//!
//! `revision` is [`Fingerprint::of_bytes`] over the file's **raw bytes**, never
//! over rendered text. A hash of rendered text is blind to exactly the things
//! this model preserves but does not represent — trailing whitespace, CRLF, an
//! `if` table's interior — so it would let a save clobber someone else's edit
//! and report success. The codebase has settled this twice already: DL-3 dropped
//! mtime from [`Fingerprint`], and the watcher's `FileStamp` records "above all
//! not any rendered text".
//!
//! # Four behaviours that look like bugs and are not
//!
//! **The tree is re-scanned on every request, and never cached.** That is layer
//! 2: an id resolves against a set built from `read_dir` names *in this
//! request*, so a cached set would be a set that no longer describes the disk,
//! and a save would be authorized by a scan that happened arbitrarily long ago.
//! The cost is real (one walk plus a parse of each discovered file, because
//! [`rules::discover`] builds list summaries) and it is the price of the
//! guarantee. Real trees are a handful of 2-8 KB files; a pathological one is
//! bounded by the scan's own entry cap.
//!
//! **A file the scan truncated away answers the ordinary `404`.** The index
//! reports `truncated`, so a client is told the list is a subset — but
//! `document`, `preview` and `save` cannot say "a cap was hit" without making
//! the `404` differ by cause, which is the one thing layer 5 forbids. An
//! indistinguishable sentence that is occasionally imprecise beats a precise one
//! that answers questions about the filesystem.
//!
//! **A file with no final newline gains one if its LAST item is edited.** The
//! engine's renderer terminates every body it re-renders, because an item that
//! lacked a terminator and stops being last would otherwise be glued onto its
//! successor — a reorder that loses no byte and changes the meaning of two
//! constructs. `Keep` is unaffected, so a client that echoes unchanged items
//! back as `keep` (which is what the whole `kind` split is for) still round-trips
//! byte-for-byte and still hits the no-op short-circuit.
//!
//! **A file whose last construct is a conditional TABLE gains a blank line when
//! something is added after it.** Same rule, different terminator: a table's
//! extent runs to the first empty line or to EOF, so one written at EOF carries
//! no terminator, and the new construct would be read back as further data rows
//! of it. The engine supplies the blank line the moment the table stops being
//! last. This is the *only* line a save adds that the client did not ask for,
//! and it is added only to a paragraph that is no longer final — so, as above, a
//! `keep`-only save of an untouched document is still byte-identical and still
//! writes nothing.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::{HeaderName, HeaderValue, header};
use axum::response::{IntoResponse, Response};
use ledgeline_core::Fingerprint;
use ledgeline_core::convert::{self, SourceFormat, Tabular};
use ledgeline_core::rules::{
    self, ControlField, CreateRefusal, DirectiveValue, DiscoveredRules, Discovery, EditPlan,
    HledgerField, IfLayout, Item, ItemBody, ItemId, ItemKind, MatchScope, MatcherGroupSpec,
    MatcherSpec, Newline, OpaqueReason, Preview, PreviewUnavailable, RulesDoc, RulesPath, Setting,
    Settings, Slot, Warning, generate,
};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path as FsPath, PathBuf};

use crate::AppState;
use crate::edit_api::json_body;
use crate::error::{AppError, editing_disabled};
use crate::reports_api::compute;
use crate::stage::{Stage, StageId};

// ===========================================================================
// Budgets
// ===========================================================================

/// How much of one item's raw text is handed out for read-only display.
///
/// Only `opaque` and `trivia` carry text at all — everything else is described
/// by typed fields — and both are shown, not edited. 4 KiB is a whole small
/// rules file, so it clips only a pathological item, and it bounds one response
/// against a document that is one enormous unparsable blob.
const MAX_ITEM_TEXT_BYTES: usize = 4096;

/// How many items a document may have — **both** on the way out and on the way
/// back.
///
/// The two directions have to share one number, and that is the whole point.
/// [`RulesDoc::apply`] requires a plan to account for every item, so a cap that
/// let a document be *read* with more items than a save may *name* would make
/// that document permanently unsavable, and would blame the client for it. So a
/// document with more items than this is refused at read time (see
/// [`parse_document`]) with the same "listed but not openable" answer an
/// over-size or non-UTF-8 file gets.
///
/// It is also what bounds a response. [`MAX_ITEM_TEXT_BYTES`] bounds one item;
/// without an item-count bound, a 1 MiB file of unclassifiable one-byte lines
/// becomes half a million items and a ~48 MB response — a 60× amplification,
/// held in memory, occupying one of the few [`compute`] slots. Real rules files
/// have tens of items, the largest fixture has dozens, and 2,000 is a hundred
/// times the biggest thing anyone hand-maintains.
const MAX_ITEMS: usize = 2_000;

/// The longest id accepted, in bytes — `PATH_MAX` on macOS, and a quarter of it
/// on Linux. An id longer than the platform's own path limit cannot name a file
/// that exists, so this refuses at no cost what the filesystem would refuse
/// anyway, and it bounds the string every error message below quotes.
const MAX_ID_BYTES: usize = 1024;

/// How many `/`-separated components an id may have.
///
/// **This is `MAX_RULES_DEPTH + 1`, and the `+ 1` is load-bearing.** The scan
/// descends eight directories below the root, so the deepest file it can return
/// has eight directory components *plus* a file name — nine. A cap of eight here
/// would refuse an id the scan itself had just handed out, which is worse than
/// useless: the file appears in the index and then cannot be opened. That
/// coupling is not visible from this file, so
/// `a_file_at_the_scan_s_maximum_depth_can_be_opened` in `rules_endpoints.rs`
/// discovers a file at the limit and then fetches it by the id the index gave.
const MAX_ID_COMPONENTS: usize = 9;

/// How many sample rows a draft's preview carries, and the two budgets that
/// bound one of them.
///
/// Deliberately more than [`rules::Preview`]'s three: this preview is the
/// mapping table itself rather than a label beside a column number, and telling
/// a month-first file from a day-first one by eye needs more than three dates.
const DRAFT_PREVIEW_ROWS: usize = 8;
const DRAFT_PREVIEW_COLUMNS: usize = 64;
const DRAFT_PREVIEW_CELL_CHARS: usize = 120;

/// The `revision` that means **"there is no file yet"**.
///
/// A `PUT` carrying it is a create; anything else is an edit against bytes that
/// exist. The empty string can never collide with a real one — [`Fingerprint`]
/// tokens are always `LEN-HASH` in hex — and it is the same spelling
/// `import_api`'s `hledger.conf` write already uses for the same fact, so the
/// SPA has one convention rather than two.
const NEW_FILE_REVISION: &str = "";

/// The largest rules file this module will read into memory.
///
/// Deliberately the same number as the discovery scan's own cap, and
/// deliberately re-stated rather than shared: this is the bound at the layer
/// that performs the read, so the route cannot be made to slurp a multi-megabyte
/// file even if the scan's cap were relaxed or its `parsed` flag were wrong.
/// Real rules files are 2-8 KB.
const MAX_DOCUMENT_BYTES: usize = 1 << 20;

// ===========================================================================
// Response wire types (native, camelCase)
// ===========================================================================

/// `GET /api/rules` — everything an imports list view needs without opening a
/// single file.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireRulesIndex {
    /// The scan root's final path component, and nothing above it — see
    /// [`Discovery::root_label`].
    root_label: String,
    /// Whether `PUT` would be accepted at all. `false` means this server has no
    /// journal file bound to an editor, so the UI should present the files
    /// read-only rather than let a user type an edit that cannot be saved.
    editable: bool,
    /// A scan cap was hit and the list is a subset. Surfaced so a missing rules
    /// file is never silently missing.
    truncated: bool,
    files: Vec<WireRulesFile>,
    /// What the walk skipped and why, each naming a **relative** path only.
    ///
    /// Plain strings rather than [`WireWarning`]s because every warning
    /// [`rules::discover`] produces is about a *file*, not a line in one: its
    /// `line` is 0 and its `item` is `None` by construction, so the two extra
    /// fields would be noise on every entry.
    warnings: Vec<String>,
}

impl WireRulesIndex {
    /// The answer for a server with no journal file open at all.
    ///
    /// Not an error: the imports screen should still render, and say why it is
    /// empty. There is no directory to scan, so there is also no root to label.
    fn without_journal() -> Self {
        Self {
            root_label: "journal".to_string(),
            editable: false,
            truncated: false,
            files: Vec::new(),
            warnings: vec![
                "this server has no journal file open, so there is no directory to look for import \
                 rules in"
                    .to_string(),
            ],
        }
    }
}

/// One discovered `*.rules` file, summarized without re-reading it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireRulesFile {
    /// The forward-slash relative path from the scan root. This IS the handle:
    /// it is what the other three routes take, and resolution is exact string
    /// equality against a freshly scanned set.
    id: String,
    label: String,
    /// [`Fingerprint::token`] over the file's raw bytes, or a non-token sentinel
    /// for a file the scan declined to read (see `parsed`).
    revision: String,
    size_bytes: u64,
    /// `false` means the scan never read the contents, so every summary field
    /// below it is empty and describes nothing. The entry exists so the file is
    /// *visible*, which beats a file that silently is not there.
    parsed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    account1: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    account2: Option<String>,
    if_block_count: usize,
    editable_block_count: usize,
    opaque_item_count: usize,
    /// What discovery noticed about **this file** — over-size, unreadable, not
    /// UTF-8. Deliberately not the file's own parse warnings: those belong to
    /// the document route, and carrying them here would make a list response
    /// scale with the contents of every file in the tree.
    warnings: Vec<String>,
}

impl From<&DiscoveredRules> for WireRulesFile {
    fn from(file: &DiscoveredRules) -> Self {
        Self {
            id: file.id.clone(),
            label: file.label.clone(),
            revision: file.revision.clone(),
            size_bytes: file.size_bytes,
            parsed: file.parsed,
            account1: file.account1.clone(),
            account2: file.account2.clone(),
            if_block_count: file.if_block_count,
            editable_block_count: file.editable_block_count,
            opaque_item_count: file.opaque_item_count,
            warnings: file
                .warnings
                .iter()
                .map(|warning| warning.message.clone())
                .collect(),
        }
    }
}

/// `GET /api/rules/{*id}` — one parsed rules file, item by item.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireRulesDoc {
    id: String,
    label: String,
    /// Echo this back in a `PUT` to prove the edit is against these bytes.
    revision: String,
    editable: bool,
    /// `"lf"` or `"crlf"`, **detected and never imposed** — a CRLF file that
    /// came back with LF terminators would show every line as changed in the
    /// user's diff.
    newline: &'static str,
    settings: WireSettings,
    items: Vec<WireItem>,
    warnings: Vec<WireWarning>,
}

/// Something hledger would probably reject, anchored to where it is.
///
/// A warning is never a refusal to open the file — showing the user what is in
/// it is the entire point of the screen.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireWarning {
    /// The item it is about, when it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    item_id: Option<u32>,
    /// 1-based line, or 0 for a warning about the file as a whole.
    line: u32,
    message: String,
}

impl From<&Warning> for WireWarning {
    fn from(warning: &Warning) -> Self {
        Self {
            item_id: warning.item.map(|id| id.0),
            line: warning.line,
            message: warning.message.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// Settings — the flattened, last-one-wins projection a preferences panel renders
// ---------------------------------------------------------------------------

/// One resolved setting: a value plus **the item that produced it**.
///
/// Carrying `itemId` is the whole point of this projection. A preferences panel
/// that edited a copy of these values would be a second source of truth, free to
/// drift out of step with the file it claims to describe; carrying the id means
/// the panel edits the real item through the same `PUT` as everything else, and
/// is a *view* rather than a copy.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WirePref<T> {
    value: T,
    item_id: u32,
}

/// The `source` setting, which needs one more field than any other.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireSource {
    /// The path or command **as written**. Never resolved, globbed or executed.
    value: String,
    /// The value contains a `|`, which makes it a shell command hledger runs on
    /// `import`. Surfaced so a UI can refuse to treat it as a path and can warn
    /// before anything runs it. Nothing in Ledgeline ever will.
    executes_shell_command: bool,
    item_id: u32,
}

/// The `fields` setting: the CSV's column names, in column order.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireFieldsPref {
    names: Vec<String>,
    item_id: u32,
}

/// What a rules file *says*, flattened.
///
/// `Option` throughout, and absent keys are omitted: "the file does not say" is
/// not the same as hledger's default for it, and choosing a default is a
/// rendering decision that belongs to whoever renders.
///
/// This covers hledger's eleven directives plus the four top-level assignments a
/// preferences panel shows. [`Settings::end`] is the one modelled setting
/// deliberately not projected here: a top-level `end` is an *assignment*, not a
/// directive, and it already appears in `items` as an ordinary `assignment` —
/// so nothing is hidden, and the panel stays a panel of directives.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<WireSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    archive: Option<WirePref<bool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    encoding: Option<WirePref<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    separator: Option<WirePref<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    decimal_mark: Option<WirePref<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    date_format: Option<WirePref<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timezone: Option<WirePref<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    newest_first: Option<WirePref<bool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    intra_day_reversed: Option<WirePref<bool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    skip: Option<WirePref<u32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    balance_type: Option<WirePref<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    account1: Option<WirePref<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    account2: Option<WirePref<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    currency: Option<WirePref<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fields: Option<WireFieldsPref>,
}

/// A `Setting<String>` as a preference entry.
fn text_pref(setting: Option<&Setting<String>>) -> Option<WirePref<String>> {
    setting.map(|setting| WirePref {
        value: setting.value.clone(),
        item_id: setting.item.0,
    })
}

/// A valueless directive (`archive`, `newest-first`, `intra-day-reversed`) as a
/// preference entry.
///
/// The value is `true` rather than `null`: the entry's *presence* is the whole
/// meaning, and a boolean is what a checkbox binds to.
fn flag_pref(setting: Option<&Setting<()>>) -> Option<WirePref<bool>> {
    setting.map(|setting| WirePref {
        value: true,
        item_id: setting.item.0,
    })
}

/// A directive value rendered exactly as [`RulesDoc::apply`] would write it.
///
/// Routing through [`rules::directive_value_text`] rather than formatting here
/// is what stops this projection from ever disagreeing with the renderer about
/// how a `separator` or a `balance-type` is spelled.
fn value_pref<T>(
    setting: Option<&Setting<T>>,
    text: impl Fn(&T) -> String,
) -> Option<WirePref<String>> {
    setting.map(|setting| WirePref {
        value: text(&setting.value),
        item_id: setting.item.0,
    })
}

impl From<&Settings> for WireSettings {
    fn from(settings: &Settings) -> Self {
        Self {
            source: settings.source.as_ref().map(|setting| WireSource {
                value: setting.value.raw.clone(),
                executes_shell_command: setting.value.has_command,
                item_id: setting.item.0,
            }),
            archive: flag_pref(settings.archive.as_ref()),
            encoding: text_pref(settings.encoding.as_ref()),
            separator: value_pref(settings.separator.as_ref(), |separator| {
                rules::directive_value_text(&DirectiveValue::Separator(separator.clone()))
            }),
            decimal_mark: value_pref(settings.decimal_mark.as_ref(), |mark| {
                rules::directive_value_text(&DirectiveValue::DecimalMark(*mark))
            }),
            date_format: text_pref(settings.date_format.as_ref()),
            timezone: text_pref(settings.timezone.as_ref()),
            newest_first: flag_pref(settings.newest_first.as_ref()),
            intra_day_reversed: flag_pref(settings.intra_day_reversed.as_ref()),
            skip: settings.skip.as_ref().map(|setting| WirePref {
                value: setting.value,
                item_id: setting.item.0,
            }),
            balance_type: value_pref(settings.balance_type.as_ref(), |kind| {
                rules::balance_type_text(*kind).to_string()
            }),
            account1: text_pref(settings.account1.as_ref()),
            account2: text_pref(settings.account2.as_ref()),
            currency: text_pref(settings.currency.as_ref()),
            fields: settings.fields.as_ref().map(|setting| WireFieldsPref {
                names: setting.value.clone(),
                item_id: setting.item.0,
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// Items
// ---------------------------------------------------------------------------

/// One paragraph of a rules file: the unit that can be reordered or deleted.
///
/// `id`, `line` and `lines` are common to every kind and live here rather than
/// being repeated seven times; `#[serde(flatten)]` puts the `kind` tag and the
/// payload beside them, so the JSON is one flat object exactly as if the enum
/// carried them.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireItem {
    /// The item's **0-based index in this parse**, and deliberately not stable
    /// across saves: a rules file has no natural key for a construct (two
    /// identical `account2 expenses:unknown` lines are indistinguishable), so a
    /// durable id would be an invented lie. Parse, plan and save against one
    /// document version; a stale id is refused rather than guessed at.
    id: u32,
    /// 1-based line of the item's **body**, which is what its warnings point at.
    line: u32,
    /// How many lines the item's whole **span** covers — leading comment run and
    /// trailing blank run included, because that is the unit a reorder moves.
    /// This is why it can exceed the body's own line count.
    lines: u32,
    #[serde(flatten)]
    body: WireItemBody,
}

/// What an item *is*, and the payload that goes with it.
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum WireItemBody {
    /// A run of comment and/or blank lines with no body to attach to.
    Trivia { text: String, truncated: bool },
    /// One of hledger's eleven rules-file directives.
    Directive {
        name: String,
        /// The value **verbatim to end of line**, trailing whitespace included,
        /// because for `date-format` it really is part of the value. Echoing it
        /// back unchanged re-emits the file's own bytes.
        value: String,
    },
    /// An `include RULESFILE` line. Present for display only — keep-only in the
    /// engine, so there is no way to write one.
    Include { target: String },
    /// A `fields` list naming the CSV's columns, in column order.
    Fields { names: Vec<String> },
    /// A top-level field assignment (`account2 expenses:unknown`).
    Assignment { field: String, value: String },
    /// A conditional block whose matchers are an OR of AND-groups and whose
    /// assignments are all well-formed — the only conditional shape that can be
    /// edited one part at a time.
    IfBlock {
        /// `"inline"` (`if MATCHER`) or `"stacked"` (a bare `if`). Preserved as
        /// found on a save, even when the matcher count changes.
        layout: &'static str,
        /// The OR-ed groups, in file order. Always at least one, each with at
        /// least one matcher — a plain OR list is one matcher per group.
        ///
        /// A group is a flat list of matchers whichever concrete syntax
        /// produced it: hledger's line-prefix `&`, its same-line `&&`, or both
        /// on one block. The spelling is the engine's to preserve; the wire
        /// carries only the grouping.
        groups: Vec<WireMatcherGroup>,
        assignments: Vec<WireAssignment>,
        /// hledger's `"skip"` (drop the matching row) or `"end"` (stop reading
        /// here), or absent. Only the bare, argument-less form reaches this;
        /// `skip N` skips N records and stays `opaque`.
        #[serde(skip_serializing_if = "Option::is_none")]
        control: Option<&'static str>,
    },
    /// A construct the engine declined to classify, carried verbatim.
    ///
    /// Opaque is never a failure — it is the honest answer that rewriting part
    /// of this construct could change what the rest means. It can still be kept,
    /// moved or deleted.
    Opaque {
        /// Which rule declined, so the UI can explain the refusal.
        reason: &'static str,
        /// A short, single-line, sanitized preview of the first body line.
        label: String,
        text: String,
        truncated: bool,
    },
}

/// One OR-branch of a conditional block: matchers AND-ed together.
///
/// The AND is hledger's own — a line-prefix `&`, or a same-line `&&` — and it
/// is carried as *nesting* rather than as text: no `&` appears in a
/// [`WireMatcher`] in either direction, and the engine's renderer is what writes
/// one. Which of the two spellings a group came from is not on the wire at all,
/// because the engine preserves the file's own bytes and only ever writes the
/// line-prefix form for a matcher it is adding.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireMatcherGroup {
    matchers: Vec<WireMatcher>,
}

/// One matcher of a conditional block.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireMatcher {
    /// The field the pattern is scoped to (`%description` → `"description"`),
    /// or absent for a whole-record matcher.
    #[serde(skip_serializing_if = "Option::is_none")]
    field: Option<String>,
    /// The regex as hledger reads it, trimmed exactly as hledger trims it. Never
    /// compiled here — this surface shows and moves patterns, it does not run
    /// them.
    pattern: String,
}

/// One assignment inside a conditional block.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireAssignment {
    field: String,
    value: String,
}

/// The name a client sees for an [`OpaqueReason`].
///
/// Spelled out rather than derived from `Debug` so a refactor in the engine
/// cannot silently rename a wire value.
const fn opaque_reason(reason: OpaqueReason) -> &'static str {
    match reason {
        OpaqueReason::IfTable => "ifTable",
        OpaqueReason::CombinedMatcher => "combinedMatcher",
        OpaqueReason::MatchGroup => "matchGroup",
        OpaqueReason::CommentLikeMatcher => "commentLikeMatcher",
        OpaqueReason::ControlFlowInBlock => "controlFlowInBlock",
        OpaqueReason::UnparsedBlockBody => "unparsedBlockBody",
        OpaqueReason::UnparsedDirective => "unparsedDirective",
        OpaqueReason::Unclassified => "unclassified",
    }
}

/// The name a client sees for a [`ControlField`], and the same table read back.
///
/// Spelled out rather than derived from `Debug`, exactly as [`opaque_reason`]
/// is, so a refactor in the engine cannot silently rename a wire value — and
/// written as one pair so the two directions cannot drift apart.
const fn control_name(control: ControlField) -> &'static str {
    match control {
        ControlField::Skip => "skip",
        ControlField::End => "end",
    }
}

/// A `control` a client asked for. `None` for absent; an unknown word is a
/// `400` rather than a silently dropped instruction to skip a row.
fn control_named(name: Option<&String>) -> Result<Option<ControlField>, AppError> {
    name.map(|name| match name.as_str() {
        "skip" => Ok(ControlField::Skip),
        "end" => Ok(ControlField::End),
        _ => Err(AppError::BadRequest(format!(
            "{} is not a conditional block control word; expected \"skip\" or \"end\"",
            quoted(name)
        ))),
    })
    .transpose()
}

/// `text` clipped to [`MAX_ITEM_TEXT_BYTES`] **on a char boundary**, plus
/// whether anything was dropped.
///
/// Clipping mid-code-point would produce a `String` that cannot exist, so the
/// cut is walked back to the nearest boundary. Offset 0 always is one, which is
/// why the fallback can never be reached.
fn clip(text: &str) -> (String, bool) {
    if text.len() <= MAX_ITEM_TEXT_BYTES {
        return (text.to_string(), false);
    }
    let end = (0..=MAX_ITEM_TEXT_BYTES)
        .rev()
        .find(|&at| text.is_char_boundary(at))
        .unwrap_or(0);
    (text[..end].to_string(), true)
}

/// How many lines a whole-line span covers. `str::lines` splits LF-only, which
/// is how the engine and `parse.rs` both number lines.
fn line_count(text: &str) -> u32 {
    u32::try_from(text.lines().count()).unwrap_or(u32::MAX)
}

fn wire_item(doc: &RulesDoc, item: &Item) -> WireItem {
    WireItem {
        id: item.id.0,
        line: item.line,
        lines: line_count(&doc.text()[item.span.clone()]),
        body: wire_item_body(doc, item),
    }
}

fn wire_item_body(doc: &RulesDoc, item: &Item) -> WireItemBody {
    // Every leaf's text is read from ITS OWN SPAN rather than re-rendered, so a
    // client that echoes an item back unchanged hands us the file's own bytes
    // and the renderer's splice is a no-op. Re-rendering here would quietly
    // normalize a value the file spells differently, and the save would look
    // like the user had edited it.
    let text = |span: &rules::Span| doc.text()[span.clone()].to_string();
    match &item.kind {
        ItemKind::Trivia => {
            let (text, truncated) = clip(&doc.text()[item.span.clone()]);
            WireItemBody::Trivia { text, truncated }
        }
        ItemKind::Directive(directive) => WireItemBody::Directive {
            name: rules::directive_keyword(directive.name).to_string(),
            value: text(&directive.value_span),
        },
        ItemKind::Include(include) => WireItemBody::Include {
            target: include.target.clone(),
        },
        ItemKind::Fields(fields) => WireItemBody::Fields {
            names: fields.names.clone(),
        },
        ItemKind::Assignment(assignment) => WireItemBody::Assignment {
            field: rules::field_name_text(assignment.field),
            value: text(&assignment.value_span),
        },
        ItemKind::IfBlock(block) => WireItemBody::IfBlock {
            layout: match block.layout {
                IfLayout::Inline => "inline",
                IfLayout::Stacked => "stacked",
            },
            groups: block
                .groups
                .iter()
                .map(|group| WireMatcherGroup {
                    matchers: group
                        .matchers
                        .iter()
                        .map(|matcher| WireMatcher {
                            field: match &matcher.scope {
                                MatchScope::Field(name) => Some(name.clone()),
                                MatchScope::WholeRecord => None,
                            },
                            pattern: matcher.pattern.clone(),
                        })
                        .collect(),
                })
                .collect(),
            assignments: block
                .assignments
                .iter()
                .map(|assignment| WireAssignment {
                    field: rules::field_name_text(assignment.field),
                    value: text(&assignment.value_span),
                })
                .collect(),
            control: block
                .control
                .as_ref()
                .map(|control| control_name(control.kind)),
        },
        ItemKind::Opaque(opaque) => {
            let (text, truncated) = clip(&doc.text()[item.span.clone()]);
            WireItemBody::Opaque {
                reason: opaque_reason(opaque.reason),
                label: opaque.label.clone(),
                text,
                truncated,
            }
        }
    }
}

fn wire_doc(
    found: &DiscoveredRules,
    doc: &RulesDoc,
    revision: String,
    editable: bool,
) -> WireRulesDoc {
    wire_doc_named(&found.id, &found.label, doc, revision, editable)
}

/// The same projection, for a document with no [`DiscoveredRules`] behind it.
///
/// The create path has one: a draft describes a file that is not on disk, so
/// there is nothing for a scan to have returned. Splitting the id and label out
/// rather than inventing a stand-in `DiscoveredRules` is what keeps that type
/// meaning "a file the scan found", which is the claim its private path field
/// exists to make.
fn wire_doc_named(
    id: &str,
    label: &str,
    doc: &RulesDoc,
    revision: String,
    editable: bool,
) -> WireRulesDoc {
    WireRulesDoc {
        id: id.to_string(),
        label: label.to_string(),
        revision,
        editable,
        newline: match doc.newline() {
            Newline::Lf => "lf",
            Newline::CrLf => "crlf",
        },
        settings: WireSettings::from(&doc.settings()),
        items: doc
            .items()
            .iter()
            .map(|item| wire_item(doc, item))
            .collect(),
        warnings: doc.warnings().iter().map(WireWarning::from).collect(),
    }
}

// ---------------------------------------------------------------------------
// Preview
// ---------------------------------------------------------------------------

/// `GET /api/rules-preview/{*id}` — the first few rows of the data file the
/// rules file describes, so a mapping screen can label `%3` with what column 3
/// actually contains.
///
/// Everything here is display-only and lossy: cells are sanitized and truncated,
/// the row count is a sample, and none of it is ever written back anywhere.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WirePreview {
    /// `false` always comes with a `reason`, and means **nothing was read**.
    available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'static str>,
    /// The data file's NAME only — never a path, and `None` when no single
    /// concrete file was ever named.
    #[serde(skip_serializing_if = "Option::is_none")]
    data_label: Option<String>,
    /// The delimiter the records below were split on. Reported because a mapping
    /// screen showing the wrong columns is far easier to debug when it also says
    /// which character it split on.
    separator: String,
    /// The record at index `skip - 1`, when the rules file has `skip >= 1`.
    #[serde(skip_serializing_if = "Option::is_none")]
    header: Option<Vec<String>>,
    rows: Vec<Vec<String>>,
    columns: usize,
    /// The read stopped at the byte cap, so this previews the file's first bytes
    /// rather than the file. Not set merely because `rows` is a sample — it
    /// always is, so a flag that was always true would say nothing.
    truncated: bool,
}

/// Why a preview has nothing to show. On every one of these, **nothing on disk
/// was read**.
const fn preview_reason(reason: PreviewUnavailable) -> &'static str {
    match reason {
        PreviewUnavailable::NoDataFile => "noDataFile",
        PreviewUnavailable::SourceIsCommand => "sourceIsCommand",
        PreviewUnavailable::SourceOutsideRoot => "sourceOutsideRoot",
        PreviewUnavailable::NotRegularFile => "notRegularFile",
        PreviewUnavailable::Unreadable => "unreadable",
        PreviewUnavailable::NotUtf8 => "notUtf8",
        PreviewUnavailable::Empty => "empty",
    }
}

impl From<&Preview> for WirePreview {
    fn from(preview: &Preview) -> Self {
        Self {
            available: preview.available,
            reason: preview.reason.map(preview_reason),
            data_label: preview.data_label.clone(),
            separator: preview.separator.to_string(),
            header: preview.header.clone(),
            rows: preview.rows.clone(),
            columns: preview.columns,
            truncated: preview.truncated,
        }
    }
}

// ---------------------------------------------------------------------------
// Create: the drafted document, and how it was arrived at
// ---------------------------------------------------------------------------

/// `POST /api/rules-create` — a starting-point rules file for a staged upload.
///
/// `doc` is the **same shape** `GET /api/rules/{*id}` returns, and `preview` the
/// same shape `GET /api/rules-preview/{*id}` returns, so the SPA renders a draft
/// through the components it already has rather than a second rendering path.
/// The two fields below them are what a draft has and a saved document does not:
/// how each column was arrived at, and what the user has to be told about it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireRulesDraft {
    doc: WireRulesDoc,
    preview: WirePreview,
    /// One entry per column of the staged table, in column order.
    columns: Vec<WireColumnGuess>,
    /// Sentences about what this draft assumed. Never a refusal — a draft is
    /// produced whatever these say, because a mapping the user can see and
    /// correct beats a `400` describing it.
    warnings: Vec<String>,
}

/// How one column was read.
///
/// The `fields` list in `doc` already says *what* each column became; this says
/// **how sure** that was, which is the half a mapping screen needs in order to
/// mark a guess as a guess rather than presenting all of them as facts.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireColumnGuess {
    index: usize,
    /// The hledger field this column assigns, or absent when nothing claimed
    /// it. Absent is a real answer — see `rules::generate`.
    #[serde(skip_serializing_if = "Option::is_none")]
    field: Option<String>,
    /// `0.0..=1.0`. Orders guesses and marks the shaky ones; nothing computes
    /// with it.
    confidence: f32,
}

/// The preview a draft carries: the staged table itself, in the shape
/// `rules-preview` already publishes.
///
/// `available` is always `true` and `reason` always absent: there is no file to
/// fail to read, because the rows come from the upload the user has already
/// made. The fields are still present, because the SPA decodes this with
/// `decodeRulesPreview` and a second, nearly-identical shape would be a second
/// decoder to keep true.
fn draft_preview(tabular: &Tabular) -> WirePreview {
    let header = tabular.header.as_ref().map(|row| clip_preview_row(row));
    let rows: Vec<Vec<String>> = tabular
        .rows
        .iter()
        .take(DRAFT_PREVIEW_ROWS)
        .map(|row| clip_preview_row(row))
        .collect();
    let columns = header
        .iter()
        .chain(rows.iter())
        .map(Vec::len)
        .max()
        .unwrap_or(0);
    WirePreview {
        available: true,
        reason: None,
        data_label: None,
        // The staged CSV is `convert::to_csv`'s output, which is always
        // comma-separated whatever the download was. Reporting the download's
        // own delimiter here would describe a file hledger will never read.
        separator: ",".to_string(),
        header,
        rows,
        columns,
        truncated: tabular.truncated,
    }
}

/// One preview row, clipped in width and per cell — the same budgets
/// `Discovery::preview` applies, restated here because this row never went
/// through it.
fn clip_preview_row(row: &[String]) -> Vec<String> {
    row.iter()
        .take(DRAFT_PREVIEW_COLUMNS)
        .map(|cell| cell.chars().take(DRAFT_PREVIEW_CELL_CHARS).collect())
        .collect()
}

// ===========================================================================
// Request wire types
// ===========================================================================

/// The `POST /api/rules-create` body.
///
/// Note what is **not** here: no column mapping, no date format, no separator.
/// A draft is the engine's own reading of the data, and the user corrects it by
/// editing the returned document and saving that — through the ordinary `PUT`,
/// against the ordinary typed item vocabulary. Accepting overrides here would be
/// a second way to say the same thing, and the two would drift.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireCreateRequest {
    /// The upload this rules file is being written for. The SAME handle the
    /// candidate list already has — the user dropped the file to get here, and
    /// re-uploading it to draft against it would be asking twice.
    stage_id: String,
    /// The id the file will have. One handle, exactly as every other rules
    /// route takes, rather than a separate directory + file name: `PUT` takes
    /// an id, `validate_id` checks an id, and a second spelling would need a
    /// second validator.
    id: String,
    /// The account this statement IS — the one thing no CSV can supply.
    ///
    /// May be empty. The draft then carries a bare `account1` line for the
    /// form to fill in, because a panel that refused to show anything until an
    /// account had been typed would be a mapping table nobody could look at.
    #[serde(default)]
    account1: String,
}

/// The `PUT /api/rules/{*id}` body: the complete intended shape of the document.
///
/// `deny_unknown_fields`, unlike the transaction write path. This is a
/// WHOLE-DOCUMENT replace, so a typo'd key must not silently mean "leave that
/// part alone" — an ignored `"delte"` would drop the deletion and report `200`.
/// The SPA and the engine ship in the same binary, so the usual
/// forward-compatibility cost of strictness is nil here.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireSaveRequest {
    /// The `revision` the document was read with. A mismatch is a `409` and
    /// nothing is written.
    revision: String,
    /// The new document, slot by slot.
    items: Vec<WireItemIn>,
    /// Items dropped on purpose. Omitting an item is NEVER an implicit delete —
    /// the engine refuses a plan that does not account for every item — so a
    /// client bug that drops half its array cannot truncate a rules file.
    #[serde(default)]
    delete: Vec<u32>,
}

/// One slot of a saved document.
///
/// **No variant carries raw text.** A client can only name typed content the
/// engine's renderers produce, or name an id whose bytes were already read from
/// that file. That is the structural half of security layer 4, and it is why
/// there is no `Trivia`, `Opaque` or `Include` variant: `Keep` is the only form
/// accepted for those, and for `source`/`archive` directives, which the engine
/// refuses to write for the remote-code-execution reason its `writable`
/// documents.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
enum WireItemIn {
    /// Emit an existing item's bytes, unchanged. Moving one is just listing it
    /// somewhere else.
    Keep { id: u32 },
    Directive {
        #[serde(default)]
        id: Option<u32>,
        name: String,
        value: String,
    },
    Fields {
        #[serde(default)]
        id: Option<u32>,
        names: Vec<String>,
    },
    Assignment {
        #[serde(default)]
        id: Option<u32>,
        field: String,
        value: String,
    },
    IfBlock {
        #[serde(default)]
        id: Option<u32>,
        groups: Vec<WireMatcherGroupIn>,
        assignments: Vec<WireAssignmentIn>,
        /// `"skip"`, `"end"`, or absent. The engine writes the bare word; there
        /// is no way to ask for `skip N` from here, which is deliberate — that
        /// skips N records and is a construct this surface does not model.
        #[serde(default)]
        control: Option<String>,
    },
}

/// One OR-branch of a saved conditional block. Mirrors [`WireMatcherGroup`].
///
/// A client says "these matchers are AND-ed" by nesting them, never by writing
/// a combinator: the engine still refuses a pattern that *starts* with `&` or
/// `!`, so the grouping is the only way to express an AND and there is no text
/// path to a `!`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WireMatcherGroupIn {
    matchers: Vec<WireMatcherIn>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WireMatcherIn {
    #[serde(default)]
    field: Option<String>,
    pattern: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WireAssignmentIn {
    field: String,
    value: String,
}

// ===========================================================================
// Security layer 1: the id, checked before any filesystem call
// ===========================================================================

/// Reject an id that cannot possibly name a discovered rules file — on SHAPE
/// alone, before anything touches the filesystem.
///
/// This is what makes the 400-vs-404 split safe. Deciding it on existence would
/// turn the route into an oracle: `../../../etc/passwd.rules` answering
/// differently depending on what is there tells an attacker what is there.
/// Deciding it on syntax tells them only what they already sent us.
///
/// It is also **not** the thing that provides confinement — [`Discovery::resolve`]
/// is, by exact string equality against a set the scan built from `read_dir`
/// names. This layer exists so that a bug in a later one is not the only thing
/// standing between a caller's string and a path.
///
/// Refused: empty; over [`MAX_ID_BYTES`]; more than [`MAX_ID_COMPONENTS`]
/// components; a leading `/` or `\`; a `\` anywhere (a Windows separator, which
/// would make `..\` a traversal on one platform and a filename on another); an
/// empty, `.` or `..` component; a `:` (a Windows drive letter, and an
/// NTFS/macOS alternate data stream); a NUL or any other ASCII control
/// character; and anything not ending in `.rules`.
fn validate_id(id: &str) -> Result<(), AppError> {
    let components: Vec<&str> = id.split('/').collect();
    let well_formed = !id.is_empty()
        && id.len() <= MAX_ID_BYTES
        && !id.starts_with('/')
        && !id.starts_with('\\')
        && !id.contains('\\')
        && !id.contains(':')
        && !id.chars().any(|c| c.is_ascii_control())
        && ends_with_rules_suffix(id)
        && components.len() <= MAX_ID_COMPONENTS
        && components
            .iter()
            .all(|part| !part.is_empty() && *part != "." && *part != "..");
    if well_formed {
        Ok(())
    } else {
        Err(malformed_id(id))
    }
}

/// Does `id` end in `.rules`, matched ASCII-case-insensitively?
///
/// Compared as BYTES: slicing a `&str` at a fixed offset from the end could land
/// mid-code-point and panic, while `[u8]` has no such hazard and `.rules` is
/// pure ASCII either way.
fn ends_with_rules_suffix(id: &str) -> bool {
    const SUFFIX: &[u8] = b".rules";
    let bytes = id.as_bytes();
    bytes.len() > SUFFIX.len() && bytes[bytes.len() - SUFFIX.len()..].eq_ignore_ascii_case(SUFFIX)
}

/// The caller's own string, escaped and clipped, ready to go in an error body.
///
/// `{:?}` escapes control characters, so a NUL or an ANSI escape sequence in a
/// hostile id reaches a terminal or a dialog as `\u{0}` rather than as itself.
/// The clip is because these strings are unbounded until [`validate_id`] has
/// run, and [`validate_id`]'s own failure message is the first thing that quotes
/// one.
fn quoted(value: &str) -> String {
    /// Long enough for any real id, short enough that a hostile one cannot make
    /// a response large.
    const MAX_QUOTED_CHARS: usize = 120;
    let clipped: String = value.chars().take(MAX_QUOTED_CHARS).collect();
    if clipped.len() < value.len() {
        format!("{clipped:?}…")
    } else {
        format!("{clipped:?}")
    }
}

/// The one `400` every syntactic rejection returns.
///
/// One sentence for all of them on purpose: the differences between them are
/// about the caller's own input, and spelling each out separately buys a client
/// nothing while giving anyone probing the route a finer-grained signal.
fn malformed_id(id: &str) -> AppError {
    AppError::BadRequest(format!(
        "{} is not a usable rules file id: an id is the file's path relative to the journal \
         directory, forward-slash separated, at most {MAX_ID_COMPONENTS} plain components and \
         {MAX_ID_BYTES} bytes, and it must end in `.rules`",
        quoted(id)
    ))
}

/// The one `404` every resolution failure returns.
///
/// **Identical for every cause** — not scanned, not there, not a regular file, a
/// symlink, outside the root, skipped by the walk — so the route cannot be used
/// to tell any of those apart. It names the caller's own id and nothing else.
fn unresolved(id: &str) -> AppError {
    AppError::NotFound(format!(
        "no rules file {} is available beside this journal",
        quoted(id)
    ))
}

/// The one `409`, shared by all three staleness checks (the revision the client
/// sent, the re-read immediately before the write, and the inode identity).
///
/// All three mean the same thing to the user and call for the same action, and
/// distinguishing them would leak the timing of somebody else's write.
fn stale(id: &str) -> AppError {
    AppError::Conflict(format!(
        "{} changed on disk since you opened it, so nothing was written. Re-open it and re-apply \
         your edit.",
        quoted(id)
    ))
}

/// A `500` for an I/O failure while READING a rules file.
///
/// The [`std::io::Error`] is surfaced verbatim, and that is safe for a
/// non-obvious reason worth writing down: **an `io::Error` that std itself
/// produced carries no path**. `File::open` failing renders as
/// `Permission denied (os error 13)` and nothing more. The path-carrying errors
/// in this codebase are ones we built deliberately (`edit.rs`'s
/// `read_journal_text` wraps one to name the journal), and none of them is on
/// this path: every error here comes straight from `File::open` or `read_to_end`.
/// The only name in the sentence is the caller's own id.
fn read_failed(id: &str, error: &std::io::Error) -> AppError {
    AppError::Internal(format!("could not read {}: {error}", quoted(id)))
}

/// A `500` for an I/O failure while WRITING a rules file — and deliberately
/// **not** [`read_failed`].
///
/// The read path's argument does not transfer, because
/// [`ledgeline_core::edit::atomic_write`] can fail with an error *we* built:
/// `create_temp_file` gives up after several collisions with
/// `could not create a temp file in {dir}` — the absolute journal directory,
/// which is precisely the disclosure layer 5 exists to prevent. Reaching it
/// needs several consecutive `AlreadyExists` on a pid-and-nanosecond-keyed temp
/// name, so it is vanishingly unlikely; an invariant that holds by luck is not
/// one worth asserting.
///
/// So only the [`std::io::ErrorKind`] is reported. Its `Display` is a fixed
/// phrase from a closed set (`permission denied`, `read-only filesystem`, `no
/// space left on device`), which keeps the half of the diagnostic a user can act
/// on and carries no payload at all.
fn write_failed(id: &str, error: &std::io::Error) -> AppError {
    AppError::Internal(format!(
        "could not write {}: {}. Nothing was changed.",
        quoted(id),
        error.kind()
    ))
}

// ===========================================================================
// Reading
// ===========================================================================

/// Read a discovered rules file's bytes, bounded and UTF-8 checked, and
/// fingerprint them.
///
/// The [`Fingerprint`] is over the **raw bytes**, before the UTF-8 decode, which
/// is the only hash a save may gate on — see the module docs.
fn read_document(found: &DiscoveredRules, id: &str) -> Result<(String, Fingerprint), AppError> {
    let file =
        std::fs::File::open(found.path().as_path()).map_err(|error| read_failed(id, &error))?;
    // `take` bounds the READ itself rather than trimming afterwards, so an
    // enormous file is never held in memory even briefly. One byte over the cap
    // is enough to detect it.
    let cap = u64::try_from(MAX_DOCUMENT_BYTES).unwrap_or(u64::MAX);
    let mut raw = Vec::new();
    file.take(cap.saturating_add(1))
        .read_to_end(&mut raw)
        .map_err(|error| read_failed(id, &error))?;
    if raw.len() > MAX_DOCUMENT_BYTES {
        return Err(AppError::BadRequest(format!(
            "{} is larger than {MAX_DOCUMENT_BYTES} bytes, so it is listed but cannot be opened \
             for editing",
            quoted(id)
        )));
    }
    let fingerprint = Fingerprint::of_bytes(&raw);
    let text = String::from_utf8(raw).map_err(|_| {
        AppError::BadRequest(format!(
            "{} is not valid UTF-8. Ledgeline reads and writes UTF-8 rules files only; converting \
             it first (e.g. `iconv -f latin1 -t utf-8`) is what keeps a character from being \
             silently rewritten.",
            quoted(id)
        ))
    })?;
    Ok((text, fingerprint))
}

/// Parse a rules file, refusing one with more items than a save could ever name.
///
/// `RulesDoc::parse` is infallible and this does not change that — the refusal
/// is about what this HTTP surface is willing to *describe*. It is the same
/// answer, and the same class of answer, as "too large" or "not UTF-8": the file
/// stays visible in the index with its summary, and it is simply not openable in
/// the editor.
///
/// Doing it here rather than only on the way in is what keeps the two directions
/// consistent. [`RulesDoc::apply`] requires a plan to account for every item, so
/// a document served with more items than [`MAX_ITEMS`] would be one the client
/// could never save — and the `400` it got back would blame it for a limit it
/// had no way to satisfy.
fn parse_document(text: &str, id: &str) -> Result<RulesDoc, AppError> {
    let doc = RulesDoc::parse(text);
    if doc.items().len() > MAX_ITEMS {
        return Err(AppError::BadRequest(format!(
            "{} has {} separate constructs, more than the {MAX_ITEMS} this editor handles, so it \
             is listed but cannot be opened for editing",
            quoted(id),
            doc.items().len()
        )));
    }
    Ok(doc)
}

/// The journal's main file — the scan root's parent, and the only thing that
/// makes a discovery set exist at all.
fn main_journal_file(state: &AppState) -> Option<PathBuf> {
    state.source_files().into_iter().next()
}

/// `Cache-Control: no-store`, no `ETag`. See the module docs for why these three
/// reads sit outside the snapshot's cache validator entirely.
fn no_store<T: Serialize>(body: T) -> Response {
    const NO_STORE: (HeaderName, HeaderValue) =
        (header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    ([NO_STORE], Json(body)).into_response()
}

// ===========================================================================
// Handlers
// ===========================================================================

/// `GET /api/rules` — every `*.rules` file in the open journal's own directory
/// tree, summarized.
pub(crate) async fn index(State(state): State<AppState>) -> Result<Response, AppError> {
    let editable = state.editing_enabled();
    let Some(main) = main_journal_file(&state) else {
        return Ok(no_store(WireRulesIndex::without_journal()));
    };
    // Through `compute`: a directory walk on a cold or network-mounted journal
    // directory is exactly the blocking work its semaphore + `spawn_blocking`
    // exist for, and running it on a tokio worker would stall the runtime the
    // desktop GUI is hosted in.
    let Json(body) = compute(move || {
        let discovery = rules::discover(&main);
        Ok(build_index(&discovery, editable))
    })
    .await?;
    Ok(no_store(body))
}

fn build_index(discovery: &Discovery, editable: bool) -> WireRulesIndex {
    WireRulesIndex {
        root_label: discovery.root_label(),
        editable,
        truncated: discovery.truncated,
        files: discovery.files.iter().map(WireRulesFile::from).collect(),
        warnings: discovery
            .warnings
            .iter()
            .map(|warning| warning.message.clone())
            .collect(),
    }
}

/// `GET /api/rules/{*id}` — one parsed rules file.
pub(crate) async fn document(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    validate_id(&id)?;
    let editable = state.editing_enabled();
    // No journal file means no scan root, so no id resolves. Answering the
    // ordinary `404` keeps this route from distinguishing "no journal open" from
    // "no such file" — the same reason every other resolution failure shares a
    // sentence.
    let Some(main) = main_journal_file(&state) else {
        return Err(unresolved(&id));
    };
    let Json(body) = compute(move || {
        let discovery = rules::discover(&main);
        let found = discovery.resolve(&id).ok_or_else(|| unresolved(&id))?;
        let (text, fingerprint) = read_document(found, &id)?;
        let doc = parse_document(&text, &id)?;
        Ok(wire_doc(found, &doc, fingerprint.token(), editable))
    })
    .await?;
    Ok(no_store(body))
}

/// `GET /api/rules-preview/{*id}` — the first few rows of the data file this
/// rules file describes.
///
/// A refusal is a value, not an error: an unavailable preview is a `200` with a
/// `reason` the GUI can explain, because "your `source` is a shell command we
/// will not run" is information, not a failure.
pub(crate) async fn preview(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    validate_id(&id)?;
    let Some(main) = main_journal_file(&state) else {
        return Err(unresolved(&id));
    };
    let Json(body) = compute(move || {
        let discovery = rules::discover(&main);
        let preview = discovery.preview(&id).ok_or_else(|| unresolved(&id))?;
        Ok(WirePreview::from(&preview))
    })
    .await?;
    Ok(no_store(body))
}

/// `PUT /api/rules/{*id}` — save a whole rules document.
///
/// The order below is the contract, and it is ordered so that **every check that
/// can fail precedes the single write**. Nothing partial can be left behind: the
/// engine renders the whole document into a `String`, verifies it, and only then
/// is one `atomic_write` performed.
pub(crate) async fn save(
    State(state): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<WireSaveRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    // 0. Syntax, before any filesystem call at all.
    validate_id(&id)?;
    // 1. The body, and the bound on how much one request may ask for.
    let request = json_body(payload)?;
    let named = request.items.len().saturating_add(request.delete.len());
    if named > MAX_ITEMS {
        return Err(AppError::BadRequest(format!(
            "a rules document may name at most {MAX_ITEMS} items; this request named {named}"
        )));
    }
    // 2. Is this server allowed to write at all? Both halves answer with the
    //    editor's own `501` sentence: a server with no journal file bound is not
    //    a server that may rewrite files beside one.
    if !state.editing_enabled() {
        return Err(editing_disabled());
    }
    let Some(main) = main_journal_file(&state) else {
        return Err(editing_disabled());
    };
    // 3. Serialize rules writes. Without this, two `PUT`s carrying the same
    //    valid revision could both pass their pre-write check and one update
    //    would be silently lost. Held across the `.await` below, which is why it
    //    is a tokio mutex and not a `std` one.
    let _write = state.rules_writes().lock().await;
    // 4. Scan, read, check, render, verify, write — all of it blocking I/O.
    let Json(body) = compute(move || save_document(&main, &id, &request)).await?;
    Ok(no_store(body))
}

/// `POST /api/rules-create` — draft a rules file for a staged upload.
///
/// **Writes nothing.** Drafting a plausible file and writing one are separate,
/// separately-testable operations: the answer is a document the SPA can show,
/// correct and only then save, through the ordinary `PUT` below. That split is
/// also what keeps this route's guards honest — nothing here can damage a file,
/// so the whole of the write-side argument lives in one place.
pub(crate) async fn create(
    State(state): State<AppState>,
    payload: Result<Json<WireCreateRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    let request = json_body(payload)?;
    // Syntax before anything else, exactly as the other four routes do. The id
    // is checked here as well as in `resolve_new` because neither layer gets to
    // assume the other ran.
    validate_id(&request.id)?;
    // A draft for a server that cannot write is a form that dead-ends on its own
    // Save button, so this route answers the editor's `501` rather than handing
    // back a document nobody could keep.
    if !state.editing_enabled() {
        return Err(editing_disabled());
    }
    let Some(main) = main_journal_file(&state) else {
        return Err(editing_disabled());
    };
    let stage = staged_upload(&state, &request.stage_id)?;
    let Json(body) = compute(move || draft_document(&main, &request, &stage)).await?;
    Ok(no_store(body))
}

/// The staged upload `raw` names, or the one `404` every stage failure shares.
///
/// Shape first, then the lookup — the same order, and for the same reason, as
/// [`validate_id`]: a handle that could not have been minted never reaches the
/// map, and "not a stage id" and "not a stage id I have" answer alike so the
/// route is not an oracle for another tab's upload.
fn staged_upload(state: &AppState, raw: &str) -> Result<std::sync::Arc<Stage>, AppError> {
    StageId::parse(raw)
        .and_then(|id| state.stages().get(&id))
        .ok_or_else(|| {
            AppError::NotFound(
                "that upload is no longer staged. Drop the file again and retry.".to_string(),
            )
        })
}

/// The whole draft, synchronously, on the blocking pool.
fn draft_document(
    main: &FsPath,
    request: &WireCreateRequest,
    stage: &Stage,
) -> Result<WireRulesDraft, AppError> {
    // Refuse an id that is taken (or unreachable) BEFORE reading a file and
    // running the generator: the user is about to fill in a form, and finding
    // out at the end that the name was never available is the worst moment to
    // be told.
    let discovery = rules::discover(main);
    discovery
        .resolve_new(&request.id)
        .map_err(|refusal| create_refused(&request.id, refusal))?;

    // The CANONICAL staged CSV — `convert::to_csv`'s own output, header on line
    // 1, comma-separated, UTF-8. Read back rather than kept in memory from the
    // upload because it is the file hledger will actually be pointed at, so a
    // mapping guessed from anything else could describe a table that differs
    // from the one being imported.
    let bytes = std::fs::read(stage.data()).map_err(|error| {
        AppError::Internal(format!(
            "could not read the staged upload: {}",
            error.kind()
        ))
    })?;
    let tabular = convert::convert(SourceFormat::Csv, &bytes)
        .map_err(|error| AppError::BadRequest(error.to_string()))?;

    let drafted = generate::generate(&tabular, request.account1.trim())?;
    Ok(WireRulesDraft {
        doc: wire_doc_named(
            &request.id,
            &rules::label_for(&request.id),
            &drafted.doc,
            NEW_FILE_REVISION.to_string(),
            true,
        ),
        preview: draft_preview(&tabular),
        columns: drafted
            .columns
            .iter()
            .map(|column| WireColumnGuess {
                index: column.index,
                field: column.field.map(rules::field_name_text),
                confidence: column.confidence,
            })
            .collect(),
        warnings: drafted.warnings.clone(),
    })
}

/// A [`CreateRefusal`] as an HTTP error.
///
/// Two of the four collapse into the ordinary `404`, and that is the whole
/// point: "outside the root" and "that directory is not there" are answers
/// about the filesystem, and a route that told them apart would be an existence
/// oracle for paths outside the journal — the exact thing security layer 5
/// exists to prevent. `Exists` is safe to report as itself, because it is only
/// reachable for a confined, non-hidden `*.rules` name below the root, which is
/// precisely the set `GET /api/rules` already publishes.
fn create_refused(id: &str, refusal: CreateRefusal) -> AppError {
    match refusal {
        CreateRefusal::Malformed => malformed_id(id),
        CreateRefusal::OutsideRoot | CreateRefusal::DirectoryMissing => unresolved(id),
        CreateRefusal::Exists => AppError::Conflict(format!(
            "{} already exists. Creating a rules file and editing one are separate actions — open \
             it from the list to change it, or choose another name.",
            quoted(id)
        )),
    }
}

/// Write a brand-new file, refusing to touch one that is already there.
///
/// `create_new` is `O_EXCL`, so the refusal is the **kernel's**, decided
/// atomically at the moment of the open. That matters more than it looks:
/// [`Discovery::resolve_new`]'s own existence check expires the instant it
/// returns, so a create that leant on it would have a window in which another
/// process could put a file there and have it silently truncated. Here the
/// window does not exist.
///
/// Deliberately **not** [`ledgeline_core::edit::atomic_write`], whose
/// temp-file-and-rename would happily replace an existing file — the property
/// that makes it right for a save is exactly what makes it wrong here.
///
/// The mode is the process umask's, unlike a save, which carries an existing
/// file's mode forward. There is no previous mode to carry, and inventing a
/// stricter one than the user's own editor would give them is a surprise rather
/// than a protection.
fn create_exclusive(path: &FsPath, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    // Before the handle drops, so "the write returned" means the bytes are on
    // the disk rather than in a cache — the same durability `atomic_write`
    // gives a save.
    file.sync_all()
}

/// The whole of a CREATE, synchronously, on the blocking pool.
///
/// Reached from [`save`] when the request carries [`NEW_FILE_REVISION`]. It
/// shares the renderer, the edit policy and [`RulesDoc::verify`] with the edit
/// path below — the document is rendered by applying the plan to an **empty**
/// one — and differs in exactly three ways, each of which is the point:
///
/// 1. resolution is [`Discovery::resolve_new`], since no scan can have found a
///    file that is not there;
/// 2. every slot must be an insert, because there are no bytes to keep;
/// 3. the write is exclusive, so creating can never become overwriting.
fn create_document(
    main: &FsPath,
    id: &str,
    request: &WireSaveRequest,
) -> Result<WireRulesDoc, AppError> {
    let discovery = rules::discover(main);
    let path: RulesPath = discovery
        .resolve_new(id)
        .map_err(|refusal| create_refused(id, refusal))?;

    let plan = plan_from_wire(request)?;
    // An empty document has no items, so `Keep`/`Replace` could only name one
    // that does not exist. `RulesDoc::apply` would refuse them anyway, with
    // "unknown item 3" — true, and useless. This says what actually happened.
    if plan.order.iter().any(|slot| slot.item_id().is_some()) || !request.delete.is_empty() {
        return Err(AppError::BadRequest(
            "a new rules file has no existing items, so every item must be a new one and there is \
             nothing to delete"
                .to_string(),
        ));
    }
    if plan.order.is_empty() {
        return Err(AppError::BadRequest(
            "a new rules file needs at least one line".to_string(),
        ));
    }

    let empty = RulesDoc::parse("");
    let new_text = empty.apply(&plan)?;
    // The same second opinion a save gets. Byte preservation is vacuous here —
    // there are no bytes to preserve — but the other half is not: `verify`
    // re-parses and requires every slot to reappear as the shape the plan said,
    // which is what catches a rendered line that reads back as something else.
    empty.verify(&plan, &new_text)?;

    create_exclusive(path.as_path(), new_text.as_bytes()).map_err(|error| {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            // Lost the race with another writer between `resolve_new` and the
            // open. The kernel refused it; this is only how that is reported.
            create_refused(id, CreateRefusal::Exists)
        } else {
            write_failed(id, &error)
        }
    })?;

    // From what we WROTE, never from a re-read — same reason the save path
    // gives: a re-read could pick up somebody else's write and hand this client
    // a token for bytes it has never seen.
    let revision = Fingerprint::of_bytes(new_text.as_bytes()).token();
    Ok(wire_doc_named(
        id,
        &rules::label_for(id),
        &RulesDoc::parse(&new_text),
        revision,
        true,
    ))
}

/// The whole save, synchronously, on the blocking pool.
///
/// Every `?` here is a decision not to write. The single [`atomic_write`] is the
/// last statement that can have an effect, and everything above it is either a
/// read or a pure computation.
///
/// [`atomic_write`]: ledgeline_core::edit::atomic_write
fn save_document(
    main: &FsPath,
    id: &str,
    request: &WireSaveRequest,
) -> Result<WireRulesDoc, AppError> {
    // A create is a different operation with a different resolution step and a
    // different write, and it says so in the request: the empty revision is the
    // revision of "there was no file". Branching on the revision rather than on
    // a separate route keeps ONE save wire for the SPA — the items, the
    // validation and the renderer are identical, and only the two ends differ.
    if request.revision == NEW_FILE_REVISION {
        return create_document(main, id, request);
    }

    // Security layer 2: a set scanned in THIS request, matched by exact string
    // equality. `root.join(id)` appears in this codebase exactly once, in
    // `Discovery::resolve_new`, which is the create path above and cannot be
    // reached from here.
    let discovery = rules::discover(main);
    let found = discovery.resolve(id).ok_or_else(|| unresolved(id))?;
    let (text, fingerprint) = read_document(found, id)?;

    // The revision is checked BEFORE any item id is resolved. A client editing
    // an older parse would otherwise be told "unknown item 7" about ids that
    // were perfectly correct when it read them, which describes the wrong
    // problem and suggests the wrong fix.
    if fingerprint.token() != request.revision {
        return Err(stale(id));
    }

    let doc = parse_document(&text, id)?;
    let plan = plan_from_wire(request)?;
    let new_text = doc.apply(&plan)?;
    // Byte preservation alone does not preserve MEANING — a conditional table
    // that lost its terminating blank line still has every byte, and silently
    // swallows the construct below it. `verify` re-parses and requires every
    // item to reappear as the same shape at the offset the plan implies.
    doc.verify(&plan, &new_text)?;

    if new_text == text {
        // NO-OP: write NOTHING. Writing byte-identical content still bumps
        // mtime, and a user's own `entr` or `hledger import` watch loop would
        // see a spurious change — the same lesson `watch_loop`'s stamp
        // comparison records (PERF-4). The unchanged document is returned, so
        // the client still gets a fresh (identical) revision.
        return Ok(wire_doc(found, &doc, fingerprint.token(), true));
    }

    // Narrow the TOCTOU window from "the whole request" to "hash → rename". It
    // cannot be closed — there is no compare-and-swap for a file — but the read
    // above happened before parsing, planning, rendering and verifying, all of
    // which take time.
    let (_, before_write) = read_document(found, id)?;
    if !before_write.content_matches(&fingerprint) {
        return Err(stale(id));
    }
    // `(dev, ino)` plus a regular-file re-check, immediately before the write.
    // The scan proved this name was a regular file inside the root; that proof
    // expired the moment the scan ended, and a name can become a symlink, a FIFO
    // or a different file entirely in between. Identity is what says "the same
    // file" rather than "a file with the same name".
    if !found.identity_unchanged() {
        return Err(stale(id));
    }

    // Reused verbatim, because every property it documents is wanted unchanged
    // here: a same-directory temp file via `create_new`, mode carry-forward (a
    // rules file's matchers carry account numbers and payee names, so `0600`
    // staying `0600` is not hypothetical), symlink-following so a shared
    // `common.rules` survives as a link, and `fsync` before `rename`.
    ledgeline_core::edit::atomic_write(found.path().as_path(), new_text.as_bytes())
        .map_err(|error| write_failed(id, &error))?;

    // The new revision comes from what we WROTE, never from a re-read: a re-read
    // could pick up somebody else's write and hand this client a token for bytes
    // it has never seen, which is the precise way to make the next save clobber
    // that person silently.
    let revision = Fingerprint::of_bytes(new_text.as_bytes()).token();
    // `RulesDoc::parse`, not `parse_document`: the write has happened, so a
    // refusal here would report failure for an edit that landed. It cannot
    // exceed the cap anyway — `verify` has already proved every non-trivia slot
    // re-parses as exactly one item, and the plan was capped at `MAX_ITEMS`.
    Ok(wire_doc(found, &RulesDoc::parse(&new_text), revision, true))
}

// ===========================================================================
// Wire → EditPlan
// ===========================================================================

/// Turn a save request into the engine's [`EditPlan`].
///
/// This is a pure translation and nothing more. Every rule about what may be
/// written — the keep-only policy for `source`/`archive`/`include`/opaque, the
/// value validation, the accounting that every item appears exactly once — lives
/// in [`RulesDoc::apply`], where it is enforced for every caller rather than for
/// this one.
fn plan_from_wire(request: &WireSaveRequest) -> Result<EditPlan, AppError> {
    Ok(EditPlan {
        order: request
            .items
            .iter()
            .map(slot_from_wire)
            .collect::<Result<Vec<_>, _>>()?,
        delete: request.delete.iter().copied().map(ItemId).collect(),
    })
}

fn slot_from_wire(item: &WireItemIn) -> Result<Slot, AppError> {
    Ok(match item {
        WireItemIn::Keep { id } => Slot::Keep(ItemId(*id)),
        WireItemIn::Directive { id, name, value } => slot(*id, directive_body(name, value)?),
        WireItemIn::Fields { id, names } => slot(
            *id,
            ItemBody::Fields {
                names: names.clone(),
            },
        ),
        WireItemIn::Assignment { id, field, value } => slot(
            *id,
            ItemBody::Assignment {
                field: field_named(field)?,
                value: value.clone(),
            },
        ),
        WireItemIn::IfBlock {
            id,
            groups,
            assignments,
            control,
        } => slot(
            *id,
            ItemBody::IfBlock {
                control: control_named(control.as_ref())?,
                groups: groups
                    .iter()
                    .map(|group| MatcherGroupSpec {
                        matchers: group.matchers.iter().map(matcher_from_wire).collect(),
                    })
                    .collect(),
                assignments: assignments
                    .iter()
                    .map(|assignment| {
                        Ok((field_named(&assignment.field)?, assignment.value.clone()))
                    })
                    .collect::<Result<Vec<_>, AppError>>()?,
            },
        ),
    })
}

/// `Some(id)` rewrites an existing item's body in place; `None` inserts a new
/// one. Nothing else distinguishes the two, which is why one function covers
/// every typed variant.
fn slot(id: Option<u32>, body: ItemBody) -> Slot {
    match id {
        Some(id) => Slot::Replace(ItemId(id), body),
        None => Slot::Insert(body),
    }
}

/// A directive named by its keyword and its value text.
///
/// Both strings go through [`rules::parse_directive`], which is the *parser's*
/// own reading of the same two pieces. A second interpretation here would be
/// free to decide that `separator TAB` or a bare `skip` means something else,
/// and both sides would still compile and pass their tests.
fn directive_body(keyword: &str, raw: &str) -> Result<ItemBody, AppError> {
    let (name, value) = rules::parse_directive(keyword, raw).ok_or_else(|| {
        AppError::BadRequest(format!(
            "{} is not one of hledger's rules-file directives, or {} is not a value it can carry",
            quoted(keyword),
            quoted(raw)
        ))
    })?;
    Ok(ItemBody::Directive { name, value })
}

/// An hledger CSV field name, read through the parser's own table.
///
/// Whether the field may be *written* where the caller put it is a separate
/// question, and the engine asks it: `skip` is a directive rather than an
/// assignment, and `end` inside a conditional block is control flow.
fn field_named(name: &str) -> Result<HledgerField, AppError> {
    rules::hledger_field(name).ok_or_else(|| {
        AppError::BadRequest(format!(
            "{} is not an hledger CSV rules field name",
            quoted(name)
        ))
    })
}

fn matcher_from_wire(matcher: &WireMatcherIn) -> MatcherSpec {
    MatcherSpec {
        scope: match &matcher.field {
            Some(field) => MatchScope::Field(field.clone()),
            None => MatchScope::WholeRecord,
        },
        pattern: matcher.pattern.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The syntactic gate in full. Every rejected shape here is one that must
    /// never reach the filesystem, and the accepted ones are exactly what a scan
    /// can produce.
    #[test]
    fn validate_id_accepts_only_ids_a_scan_could_have_produced() {
        for id in [
            "checking.csv.rules",
            "import/2026/bank.csv.rules",
            // Nine components: eight directories, which is the deepest the scan
            // descends, plus the file name. See `MAX_ID_COMPONENTS`.
            "a/b/c/d/e/f/g/h/deep.rules",
            "Checking.RULES",
            "kaffee-über.rules",
        ] {
            assert!(validate_id(id).is_ok(), "{id} should be accepted");
        }

        for id in [
            "",                           // empty
            "../escape.rules",            // traversal
            "a/../b.rules",               // traversal, mid-path
            "./a.rules",                  // a `.` component
            "/etc/passwd.rules",          // absolute
            "\\\\server\\share\\a.rules", // UNC / Windows separators
            "a\\b.rules",                 // a `\` anywhere
            "C:/x.rules",                 // drive letter
            "a.rules:stream",             // alternate data stream (also no suffix)
            "a::$DATA.rules",             // NTFS stream, suffix restored
            "x.txt",                      // not a rules file
            ".rules",                     // the suffix is not a name
            "a//b.rules",                 // empty component
            "a/b/c/d/e/f/g/h/i/j.rules",  // ten components, one past the scan's reach
            "a\u{0}.rules",               // NUL
            "a\n.rules",                  // newline
            "a\u{7}.rules",               // bell
        ] {
            assert!(validate_id(id).is_err(), "{id:?} should be rejected");
        }

        // The length cap, checked on the byte length rather than the char count.
        let long = format!("{}.rules", "a".repeat(MAX_ID_BYTES));
        assert!(validate_id(&long).is_err());
    }

    /// A hostile id must not be able to put a control character, or a megabyte
    /// of text, into a user-facing error body.
    #[test]
    fn quoted_escapes_control_characters_and_clips() {
        assert_eq!(quoted("a\u{0}b"), "\"a\\0b\"");
        assert_eq!(quoted("a\nb"), "\"a\\nb\"");
        let huge = "x".repeat(10_000);
        let rendered = quoted(&huge);
        assert!(rendered.len() < 200, "an error body must stay small");
        assert!(
            rendered.ends_with('…'),
            "a clip must be visible: {rendered}"
        );
    }

    /// Clipping a multi-byte character in half would produce a `String` that
    /// cannot exist, so the cut walks back to a boundary.
    #[test]
    fn clip_never_splits_a_code_point() {
        let text = "é".repeat(MAX_ITEM_TEXT_BYTES);
        let (clipped, truncated) = clip(&text);
        assert!(truncated);
        assert!(clipped.len() <= MAX_ITEM_TEXT_BYTES);
        assert!(text.starts_with(&clipped));

        let (whole, truncated) = clip("short");
        assert_eq!(whole, "short");
        assert!(!truncated);
    }

    /// `line` and `lines` answer different questions, and a paragraph with a
    /// leading comment run is where the difference shows.
    #[test]
    fn an_items_line_is_its_body_and_its_line_count_is_its_whole_span() {
        let doc = RulesDoc::parse("# why\nskip 1\n\nfields date, amount\n");
        let items = doc.items();
        let first = wire_item(&doc, &items[0]);
        assert_eq!(first.line, 2, "`line` points at the body, not the comment");
        assert_eq!(
            first.lines, 3,
            "`lines` covers the comment, the body and the trailing blank"
        );
    }
}
