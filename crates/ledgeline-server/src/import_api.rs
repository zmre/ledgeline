//! The HTTP surface for **New Transactions** — dropping a statement file and
//! turning it into journal entries (WP-11 lane E).
//!
//! Six routes, and this module is the only place they are wired:
//!
//! - `GET  /api/import/capabilities` — may this screen offer anything at all?
//! - `POST /api/import/stage`        — raw bytes in, preview + ranked rules out.
//! - `POST /api/import/dry-run`      — what would be imported, and what it costs.
//! - `POST /api/import/commit`       — write the CSV, run the import, report.
//! - `POST /api/import/save-csv`     — write the CSV and nothing else.
//! - `POST /api/import/sort`         — the confirmed format-preserving re-sort.
//! - `GET`/`PUT /api/prefs`          — the preferences store.
//!
//! `save-csv` is its own route rather than a `dry-run`/`commit` with null
//! handles. "No rules file fits this statement, keep the converted CSV anyway" is
//! a real state the spec requires the screen to offer, but a dry-run with no
//! rules file is *meaningless* — there is nothing to propose and nothing to
//! reconcile — so making `rulesId` and `journalId` nullable would encode a state
//! that cannot happen and would put a `null` check in front of every use of
//! either. A separate route with a two-field body cannot express it at all.
//!
//! Like [`rules_api`](crate::rules_api), this is deliberately thin: every
//! decision that could damage a user's file was made in the engine
//! ([`convert`](ledgeline_core::convert), [`matching`](ledgeline_core::rules::matching),
//! [`sort`](ledgeline_core::sort), [`journals`](ledgeline_core::journals)) or in
//! the two subprocess modules ([`hledger`](crate::hledger), [`git`](crate::git)),
//! where it is unit-testable. What lives here is **sequencing**, and the
//! sequencing is the part that is easy to get subtly and silently wrong.
//!
//! # The five sequencing rules the SERVER owns
//!
//! None of these may be left to the UI. A browser is not a security boundary,
//! and three of the five are wrong-answer bugs rather than error bugs — they
//! produce a plausible number nobody questions.
//!
//! 1. **A stage is materialised under the destination's own file name, with the
//!    destination's `.latest.NAME` copied in beside it.** `hledger import`
//!    de-duplicates from a state file kept next to the *data file* and keyed to
//!    its name. A dry-run against a temp copy called anything else consults a
//!    state file that does not exist, reports every row as new, and then the real
//!    import silently drops the back-dated ones. See [`stage`](crate::stage).
//! 2. **`commit` writes the CSV to its FINAL destination first, then imports
//!    there**, so hledger writes `.latest` next to the file it will look for next
//!    time. Importing the temp copy and moving the CSV afterwards leaves the
//!    dedup state in a directory that is about to be deleted.
//! 3. **A non-empty `blockedByGit` makes `commit` refuse**, re-checked here
//!    rather than trusted from the dry-run's response. The whole value of the git
//!    safety net is that the pre-import state was committed; a client that skips
//!    the check must not be able to skip the guarantee.
//! 4. **Balance verification concatenates. Never two `-f` flags.** This is fact 3
//!    in `plans/11-enhanced-import.md` and it is the sharpest edge in the whole
//!    feature — see [`verify_balance`].
//! 5. **A balance assertion carries its commodity, and is refused before the
//!    import is applied.** A bare `= 2949.80` asserts a balance in the *empty*
//!    commodity, computes 0 and fails forever — fact 4 again, in our own output.
//!    See [`assertion_lines`] for the commodity, [`preflight_assertion`] for the
//!    ordering.
//!
//! # Security: the same five layers, one route at a time
//!
//! `POST /api/import/commit` is a **write-anywhere primitive** if any of its
//! three client-supplied handles can be turned into an arbitrary path. So:
//!
//! 1. **Syntactic validation before any filesystem call** ([`validate_relative_id`],
//!    [`bare_filename`], [`stage::StageId::parse`]) — decided on shape, so no
//!    route is an existence oracle.
//! 2. **Set membership, not path arithmetic.** A `rulesId` resolves only through
//!    [`Discovery::resolve`]; a `journalId` only by exact equality against
//!    [`journals::targets`] over the journal *this server has open*; a `stageId`
//!    only through [`StageArea::get`]. None of the three is joined onto a root to
//!    produce a path that is then used.
//! 3. **Confinement, file type, symlinks.** `csvPath` is the one handle that
//!    names a file which may not exist yet, so it cannot be resolved by
//!    membership; [`resolve_destination`] canonicalizes its *parent* and requires
//!    the result to sit inside the include root, then refuses a target that
//!    exists as anything but a regular file.
//! 4. **Content provenance.** The only bytes written to a CSV destination are
//!    [`convert::to_csv`] output over a staged upload; the only bytes appended to
//!    a journal are hledger's own, plus (on request) one assertion transaction
//!    built from typed fields and verified by `hledger check` *before* it is
//!    written.
//! 5. **No absolute path is ever echoed.** hledger and git both put absolute
//!    paths in their diagnostics — `imported N new transactions from bank.csv to
//!    /Users/…/main.journal` is real output — and those diagnostics are far too
//!    useful to withhold, so they are passed through a [`Redactor`] that rewrites
//!    the paths *we* supplied back into the relative handles the client already
//!    has. Same trade, and the same mechanism, as `git.rs`'s `scrub`.
//!
//! # Why nothing here is cached in [`AppState`]
//!
//! Neither the resolved `hledger` nor the loaded [`Prefs`] is held across
//! requests. Both are cheap (one `stat`-guarded spawn and one small file read),
//! and both describe *the machine*, not the journal: a user who installs hledger,
//! or points the preference at a different binary, must not be told for the rest
//! of the session that nothing changed. `git::git_available` documents the same
//! decision for the same reason.

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::{HeaderMap, HeaderName, HeaderValue, header};
use axum::response::{IntoResponse, Response};
use ledgeline_core::aliases;
use ledgeline_core::convert::{
    self, ConvertError, ConvertNote, SourceFormat, StatementMeta, Tabular,
};
use ledgeline_core::decimal::Dec;
use ledgeline_core::journals::{self, JournalTarget};
use ledgeline_core::rules::matching::{self, Candidate, Ranking, Score, Signals};
use ledgeline_core::rules::{self, Discovery, RulesDoc};
use ledgeline_core::sort;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::AppState;
use crate::alias_api::WireAlias;
use crate::edit_api::json_body;
use crate::error::AppError;
use crate::git::{GitStatus, Repo};
use crate::hledger::{Hledger, HledgerError, Invocation, Output};
use crate::prefs::{self, Prefs};
use crate::reports_api::compute;
use crate::stage::{self, RUN_BARE, RUN_WITH_LATEST, Stage, StageId};

// ===========================================================================
// Budgets
// ===========================================================================

/// The upload header carrying the dropped file's name.
const FILENAME_HEADER: &str = "x-ledgeline-filename";

/// How many preview rows one stage response carries.
///
/// The preview exists so a user can see that the conversion read their file the
/// way they expected; twenty rows answers that, and a statement with twelve
/// thousand rows must not turn into a twelve-thousand-row JSON body on the way
/// back to a browser that will render the first screenful.
const PREVIEW_ROWS: usize = 20;

/// How many columns of a preview row are carried, and how many characters of
/// each cell. Together these bound one preview at a few kilobytes regardless of
/// what the upload contained.
const PREVIEW_COLUMNS: usize = 64;
/// See [`PREVIEW_COLUMNS`].
const PREVIEW_CELL_CHARS: usize = 200;

/// How many rules files are read and pre-filtered per upload.
///
/// The discovery scan already caps what it returns; this bounds the *parse* on
/// top of it, because stage 1 has to read and parse each candidate before it can
/// reject it. Two hundred small files is a fraction of a second and is what
/// discovery already permits.
const MAX_PREFILTERED: usize = 200;

/// Longest client-supplied relative handle accepted, in bytes. `PATH_MAX` on
/// macOS; a handle longer than the platform's own limit cannot name a file that
/// exists. Same number, and the same argument, as `rules_api::MAX_ID_BYTES`.
const MAX_ID_BYTES: usize = 1024;

/// How many `/`-separated components a handle may have. Matches
/// `rules_api::MAX_ID_COMPONENTS`, which is the discovery scan's depth plus the
/// file name — a cap below that would refuse a handle the scan itself produced.
const MAX_ID_COMPONENTS: usize = 9;

/// Longest accepted `X-Ledgeline-Filename`, in bytes. Comfortably past any real
/// file name and short enough that a hostile header cannot make a response large
/// or a directory entry unwriteable.
const MAX_FILENAME_BYTES: usize = 255;

/// Longest accepted statement balance / account string. Both are echoed into
/// error messages and one of them is written into a journal, so both are bounded
/// before they are used.
const MAX_FIELD_CHARS: usize = 128;

/// Wall-clock budget for an `import` or `print` run over a whole statement.
///
/// Longer than [`hledger::DEFAULT_TIMEOUT`](crate::hledger::DEFAULT_TIMEOUT)
/// because a real import parses the target journal as well as the CSV, and a
/// large journal on a cold page cache is legitimately slow. Still finite: the
/// desktop window must not be hostage to a hung child.
const IMPORT_TIMEOUT: Duration = Duration::from_secs(120);

/// Wall-clock budget for the two balance verifications, which parse the journal
/// through a pipe and compute one account's running total.
const BALANCE_TIMEOUT: Duration = Duration::from_secs(60);

// ===========================================================================
// Response wire types (native, camelCase)
// ===========================================================================

/// `GET /api/import/capabilities` — what the screen may offer at all.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireCapabilities {
    hledger: WireHledger,
    formats: Vec<&'static str>,
    journals: Vec<WireJournal>,
    git: WireGit,
    /// Every `alias` directive the journal declares, in file order, each saying
    /// whether it will be forwarded to this import and why not.
    ///
    /// Here rather than only on the dry-run because "what is in force" is a
    /// property of the journal, not of one upload — and because an alias the
    /// user wrote and Ledgeline refused needs somewhere to be visible even when
    /// nothing has been dropped on the screen yet.
    aliases: Vec<WireAlias>,
    /// `false` means no journal is bound to an editor, so nothing here can be
    /// written; the UI renders read-only and says why.
    editable: bool,
}

/// Whether a usable `hledger` was found, and if not, precisely why.
///
/// `reason` is a closed set the UI switches on; `message` is the sentence it
/// shows. Both are present exactly when `available` is `false`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireHledger {
    available: bool,
    /// hledger's own `MAJOR.MINOR` display, matching what `--version` prints.
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

impl From<&Result<Hledger, HledgerError>> for WireHledger {
    fn from(resolved: &Result<Hledger, HledgerError>) -> Self {
        match resolved {
            Ok(hledger) => Self {
                available: true,
                version: Some(hledger.version().to_string()),
                reason: None,
                message: None,
            },
            Err(error) => Self {
                available: false,
                version: None,
                reason: Some(hledger_reason(error)),
                message: Some(error.to_string()),
            },
        }
    }
}

/// The machine-readable half of [`WireHledger`].
///
/// Spelled out rather than derived from `Debug`, so a refactor in `hledger.rs`
/// cannot silently rename a wire value. `timedOut` is not in the WP-11 contract's
/// original list — it arrived with `HledgerError::TimedOut`, which exists because
/// a hung binary and a missing one call for completely different advice.
const fn hledger_reason(error: &HledgerError) -> &'static str {
    match error {
        HledgerError::NotFound => "notFound",
        HledgerError::TooOld { .. } => "tooOld",
        HledgerError::Unrunnable => "unrunnable",
        HledgerError::TimedOut { .. } => "timedOut",
    }
}

/// One journal file an import could be written to.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireJournal {
    id: String,
    label: String,
    txn_count: usize,
    last_txn_date: Option<String>,
    is_root: bool,
    writable: bool,
}

impl From<&JournalTarget> for WireJournal {
    fn from(target: &JournalTarget) -> Self {
        Self {
            id: target.id.clone(),
            label: target.label.clone(),
            txn_count: target.txn_count,
            last_txn_date: target.last_txn_date.clone(),
            is_root: target.is_root,
            writable: target.writable,
        }
    }
}

/// Whether the import can commit around itself, and whether it is meant to.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireGit {
    available: bool,
    autocommit: bool,
}

/// `POST /api/import/stage` — the converted upload, ranked against the user's
/// rules files.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireStage {
    /// Opaque. NOT a path, and not resolvable to one — see [`crate::stage`].
    stage_id: String,
    format: &'static str,
    preview: WirePreview,
    statement: Option<WireStatement>,
    notes: Vec<WireNote>,
    candidates: Vec<WireCandidate>,
    defaults: WireDefaults,
}

/// The first rows of the converted CSV, clipped for display.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WirePreview {
    header: Option<Vec<String>>,
    rows: Vec<Vec<String>>,
    /// Every data row the conversion produced, not just the ones carried here.
    row_count: usize,
    /// The CONVERSION hit a cap, so `rowCount` is itself a lower bound. Not set
    /// merely because `rows` is a sample — it always is.
    truncated: bool,
}

/// What the format volunteered about the statement as a whole.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireStatement {
    account_hint: Option<String>,
    currency: Option<String>,
    ledger_balance: Option<String>,
    balance_as_of: Option<String>,
}

impl From<&StatementMeta> for WireStatement {
    fn from(meta: &StatementMeta) -> Self {
        Self {
            account_hint: meta.account_hint.clone(),
            currency: meta.currency.clone(),
            ledger_balance: meta.ledger_balance.clone(),
            balance_as_of: meta.balance_as_of.clone(),
        }
    }
}

/// A judgement the conversion made, tagged by `kind` so the UI can phrase each
/// one properly rather than showing a pre-rendered sentence.
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum WireNote {
    SheetChosen { name: String, of: usize },
    DatesFromSerial { count: usize },
    EncodingGuessed { label: String },
    DelimiterSniffed { delimiter: String },
    PreambleSkipped { lines: usize },
    RaggedRows { count: usize },
    BalanceMismatch { expected: String, computed: String },
}

impl From<&ConvertNote> for WireNote {
    fn from(note: &ConvertNote) -> Self {
        match note {
            ConvertNote::SheetChosen { name, of } => Self::SheetChosen {
                name: name.clone(),
                of: *of,
            },
            ConvertNote::DatesFromSerial { count } => Self::DatesFromSerial { count: *count },
            ConvertNote::EncodingGuessed { label } => Self::EncodingGuessed {
                label: label.clone(),
            },
            ConvertNote::DelimiterSniffed { delimiter } => Self::DelimiterSniffed {
                delimiter: delimiter.to_string(),
            },
            ConvertNote::PreambleSkipped { lines } => Self::PreambleSkipped { lines: *lines },
            ConvertNote::RaggedRows { count } => Self::RaggedRows { count: *count },
            ConvertNote::BalanceMismatch { expected, computed } => Self::BalanceMismatch {
                expected: expected.clone(),
                computed: computed.clone(),
            },
        }
    }
}

/// One scored rules file.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireCandidate {
    id: String,
    label: String,
    score: f32,
    signals: WireSignals,
    /// The first couple of transactions this rules file would produce, so the
    /// user can recognise their own data rather than trusting a number.
    sample: Vec<WireProposed>,
    /// The rules file's own top-level `account1` — the account every imported
    /// posting lands in, and therefore the only account a statement balance
    /// could sensibly be asserted against. The screen defaults the balance
    /// account to it.
    ///
    /// **Contract amendment.** These two are a projection of fields
    /// [`DiscoveredRules`](ledgeline_core::rules::DiscoveredRules) already
    /// carries, and they are here because without
    /// them the SPA had to fetch the whole of `/api/rules` and join it onto this
    /// list by `id` to fill in one text box — a second round trip, a second
    /// scan of the journal directory, and a join whose failure mode is a
    /// silently empty field.
    #[serde(skip_serializing_if = "Option::is_none")]
    account1: Option<String>,
    /// The file's top-level `account2`, on the same terms. Carried for the same
    /// reason and in the same breath: the pair is what the rules file says the
    /// import is *between*, and a screen that shows one and not the other invites
    /// the question.
    #[serde(skip_serializing_if = "Option::is_none")]
    account2: Option<String>,
}

/// The evidence behind a score. Carried so the UI can say *why*: "4 postings
/// would have no amount" is actionable where "0.18" is not.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireSignals {
    txns: usize,
    postings: usize,
    amountless_postings: usize,
    bare_commodity_amounts: usize,
    unknown_accounts: usize,
}

impl From<&Signals> for WireSignals {
    fn from(signals: &Signals) -> Self {
        Self {
            txns: signals.txns,
            postings: signals.postings,
            amountless_postings: signals.amountless_postings,
            bare_commodity_amounts: signals.bare_commodity_amounts,
            unknown_accounts: signals.unknown_accounts,
        }
    }
}

/// One sample transaction, flattened for display.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireProposed {
    date: String,
    description: String,
    postings: Vec<String>,
}

/// What the form starts out filled in with. Both are relative handles.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireDefaults {
    csv_path: String,
    journal_id: Option<String>,
}

/// `POST /api/import/dry-run` — the two shapes of the contract, as one type.
///
/// Untagged, so a success serialises as the full object and a failure as exactly
/// `{"ok": false, "stderr": "…"}`. `ok` is the discriminator the client reads;
/// there is no second tag, because a second tag would be a second thing that can
/// disagree with the first.
#[derive(Serialize)]
#[serde(untagged)]
enum WireDryRun {
    Proposed(Box<WireProposal>),
    Failed(WireDryRunFailed),
}

/// A dry-run hledger completed.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireProposal {
    /// Always `true`.
    ok: bool,
    /// hledger's **stdout**, verbatim (paths redacted): valid, re-parseable
    /// journal text. Never scraped for the entries themselves.
    entries: String,
    count: usize,
    /// hledger's **stderr**, verbatim (paths redacted): the
    /// `would import N new transactions from FILE:` status line.
    status: String,
    /// Rows `.latest` dedup would silently drop, or `null` when there is no
    /// dedup state to drop anything.
    skipped: Option<WireSkipped>,
    balance: Option<WireBalance>,
    /// What the journal's `alias` directives did to *these* entries, or `null`
    /// when none is in force. See [`alias_effect`].
    aliases: Option<WireAliasEffect>,
    /// Targets git reports as modified. Non-empty means `commit` will refuse.
    blocked_by_git: Vec<String>,
}

/// The account rewrites the forwarded aliases performed on this import.
///
/// **Measured, not inferred.** The same dry-run is repeated with no `--alias` at
/// all and the two proposals are compared posting by posting, so the renames are
/// hledger's own answer rather than our reimplementation of its regex engine —
/// which the parser explicitly declines to attempt (see `parse.rs`). It is the
/// technique [`skipped_by_dedup`] already uses, for the same reason: the only
/// way to be sure what a subprocess did is to ask it twice.
///
/// It costs one extra `import --dry-run`, and only when the journal actually
/// declares an alias, which almost none do.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireAliasEffect {
    /// How many aliases were handed to hledger for this import.
    forwarded: usize,
    /// The distinct account renames they caused here, in first-seen order.
    /// **Empty means the aliases matched nothing in this statement**, which is
    /// the UI's cue to stay quiet.
    renames: Vec<WireRename>,
}

/// One account rewrite an alias performed.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireRename {
    /// The account the rules file produced.
    from: String,
    /// The account the import will actually write.
    to: String,
}

/// A dry-run hledger refused. `stderr` is rendered verbatim in a `<pre>` — it
/// carries the `record:` echo that says which row broke, which no paraphrase of
/// ours would.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireDryRunFailed {
    /// Always `false`.
    ok: bool,
    stderr: String,
}

/// What `.latest` dedup would drop, and from when.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireSkipped {
    older_than: String,
    count: usize,
}

/// The statement-balance reconciliation.
///
/// **All three amounts are in ONE representation** — the commodity hledger
/// reported for the account, applied to every field, both numerals at the same
/// scale. They are shown side by side, so `2949.80` (what the user typed) beside
/// `$2949.80` (what hledger answered) read as a mismatch at a glance even when
/// `matches` was `true`. See [`reconcile`].
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireBalance {
    /// What the user typed, rendered in the commodity and at the scale of
    /// `computed`.
    statement: String,
    /// What hledger computed over journal + proposed.
    computed: String,
    matches: bool,
    /// `statement - computed`, in the same commodity, or `null` when either side
    /// is not a number this engine can subtract (a multi-commodity balance, most
    /// likely).
    difference: Option<String>,
}

/// `POST /api/import/commit` — what was written.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireCommit {
    csv_written: String,
    journal_written: String,
    imported: usize,
    ordering: WireOrdering,
    /// `null` when neither target is under version control.
    git: Option<WireGitResult>,
}

/// `POST /api/import/save-csv` — the CSV was kept, and nothing else happened.
///
/// Deliberately a subset of [`WireCommit`] rather than the same type with holes
/// in it: there is no journal, no count and no ordering on this path, and
/// sending `journalWritten: null, imported: 0, ordering: {inOrder: true}` would
/// be three fields whose only job is to be ignored. The SPA's `CommitResult`
/// decoder already reads a missing `journalWritten` as "no journal was touched".
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireSavedCsv {
    csv_written: String,
    /// `null` when the destination is not under version control.
    git: Option<WireGitResult>,
}

/// Whether the journal is still in date order after the import, and what a
/// re-sort would move.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireOrdering {
    in_order: bool,
    moves: Vec<WireMove>,
}

/// One transaction a re-sort would move.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireMove {
    date: String,
    description: String,
    from_line: u32,
    to_line: u32,
}

impl From<&sort::Move> for WireMove {
    fn from(moved: &sort::Move) -> Self {
        Self {
            date: moved.date.clone(),
            description: moved.description.clone(),
            from_line: moved.from_line,
            to_line: moved.to_line,
        }
    }
}

/// What the git safety net did.
///
/// **Contract amendment:** `message` is not in the WP-11 wire table. `git.rs`
/// requires that a failed commit "surfaces its stderr in the result panel", and
/// the specified object — `{committed, paths, skipped}` — has nowhere to put it,
/// so a rejecting pre-commit hook would have reached the user as a silent
/// `committed: false`. Additive and omitted on success.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireGitResult {
    committed: bool,
    /// Paths actually committed, relative to their repository toplevel.
    paths: Vec<String>,
    /// Targets not committed: outside any repository, or gitignored.
    skipped: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

/// `POST /api/import/sort` — how many transactions moved.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireSorted {
    moved: usize,
}

/// `GET`/`PUT /api/prefs`.
///
/// `hledgerPath` is the one place an absolute path legitimately appears in an
/// `/api/*` body: it is the caller's own value, echoed back so the settings form
/// can show what is stored. Nothing here is a path *this server resolved*.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WirePrefs {
    #[serde(default)]
    hledger_path: Option<String>,
    #[serde(default)]
    git_autocommit: Option<bool>,
}

impl From<&Prefs> for WirePrefs {
    fn from(prefs: &Prefs) -> Self {
        Self {
            hledger_path: prefs
                .hledger_path
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
            git_autocommit: prefs.git_autocommit,
        }
    }
}

// ===========================================================================
// Request wire types
// ===========================================================================

/// The `dry-run` body, and — with one more field — the `commit` body.
///
/// `deny_unknown_fields`, like the rules save path and for the same reason: the
/// SPA and this server ship in one binary, so strictness costs nothing, and a
/// typo'd key silently meaning "use the default" is exactly how a commit lands
/// somewhere nobody asked for.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireDryRunRequest {
    stage_id: String,
    rules_id: String,
    csv_path: String,
    journal_id: String,
    #[serde(default)]
    balance: Option<String>,
    #[serde(default)]
    balance_account: Option<String>,
}

/// The `commit` body: the dry-run's, plus whether to write the statement balance
/// into the journal as an assertion.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireCommitRequest {
    #[serde(flatten)]
    plan: WireDryRunRequest,
    #[serde(default)]
    write_assertion: bool,
}

/// The `save-csv` body: a staged upload and where to keep it.
///
/// **Two fields, and neither of them is nullable.** This is the whole of the
/// "no rules file fits, keep the CSV anyway" path, which the spec requires the
/// screen to offer. The alternative — making `rulesId` and `journalId` optional
/// on [`WireDryRunRequest`] — would let a caller ask for a dry-run with nothing
/// to run, a commit with nothing to import, and a balance assertion against no
/// journal; three states that cannot happen, each needing a `null` check at
/// every use. A body that cannot express them is the cheaper guarantee.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireSaveCsvRequest {
    stage_id: String,
    csv_path: String,
}

/// The `sort` body.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireSortRequest {
    journal_id: String,
}

// ===========================================================================
// Security layer 1: shape, before any filesystem call
// ===========================================================================

/// The caller's own string, escaped and clipped, ready for an error body.
///
/// `{:?}` escapes control characters, so a NUL or an ANSI escape sequence in a
/// hostile handle reaches a terminal or a dialog as `\u{0}` rather than as
/// itself. Byte-for-byte the argument `rules_api::quoted` makes.
fn quoted(value: &str) -> String {
    /// Long enough for any real handle, short enough that a hostile one cannot
    /// make a response large.
    const MAX_QUOTED_CHARS: usize = 120;
    let clipped: String = value.chars().take(MAX_QUOTED_CHARS).collect();
    if clipped.len() < value.len() {
        format!("{clipped:?}…")
    } else {
        format!("{clipped:?}")
    }
}

/// Reject a relative handle that could not name a file inside the journal
/// directory — on SHAPE alone, before anything touches the filesystem.
///
/// Refused: empty; over [`MAX_ID_BYTES`]; more than [`MAX_ID_COMPONENTS`]
/// components; a leading `/`; a `\` anywhere (a Windows separator, which would
/// make `..\` a traversal on one platform and a filename on another); an empty,
/// `.` or `..` component; a `:` (a Windows drive letter, and an NTFS/macOS
/// alternate data stream); any ASCII control character; and anything not ending
/// in `suffix` when one is required.
///
/// **This is not what provides confinement.** For `rulesId` and `journalId` that
/// is set membership; for `csvPath` it is [`resolve_destination`]. This layer
/// exists so that a bug in one of those is not the only thing standing between a
/// caller's string and a path.
fn validate_relative_id(id: &str, what: &str, suffix: Option<&str>) -> Result<(), AppError> {
    let components: Vec<&str> = id.split('/').collect();
    let well_formed = !id.is_empty()
        && id.len() <= MAX_ID_BYTES
        && !id.starts_with('/')
        && !id.contains('\\')
        && !id.contains(':')
        && !id.chars().any(|c| c.is_ascii_control())
        && components.len() <= MAX_ID_COMPONENTS
        && components
            .iter()
            .all(|part| !part.is_empty() && *part != "." && *part != "..")
        && suffix.is_none_or(|suffix| ends_with_suffix(id, suffix));
    if well_formed {
        return Ok(());
    }
    let requirement = suffix.map_or_else(String::new, |suffix| {
        format!(", and it must end in `{suffix}`")
    });
    Err(AppError::BadRequest(format!(
        "{} is not a usable {what}: it is a path relative to the journal directory, \
         forward-slash separated, at most {MAX_ID_COMPONENTS} plain components and \
         {MAX_ID_BYTES} bytes{requirement}",
        quoted(id)
    )))
}

/// Does `id` end in `suffix`, matched ASCII-case-insensitively?
///
/// Compared as BYTES: slicing a `&str` at a fixed offset from the end could land
/// mid-code-point and panic, while `[u8]` has no such hazard and every suffix
/// here is pure ASCII.
fn ends_with_suffix(id: &str, suffix: &str) -> bool {
    let (bytes, suffix) = (id.as_bytes(), suffix.as_bytes());
    bytes.len() > suffix.len() && bytes[bytes.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
}

/// The upload's `X-Ledgeline-Filename`, as a bare name.
///
/// **Refused, not silently stripped.** A header of `../../.bashrc` is either an
/// attack or a bug; quietly treating it as `.bashrc` and carrying on hides both.
/// The name is used for format detection and for the destination default, so it
/// is validated before it is used *for anything*.
fn bare_filename(headers: &HeaderMap) -> Result<String, AppError> {
    let malformed = |raw: &str| {
        AppError::BadRequest(format!(
            "{} is not a usable file name: {FILENAME_HEADER} must carry a single plain file name, \
             with no directory separators, no `..`, and no control characters",
            quoted(raw)
        ))
    };
    let raw = headers
        .get(HeaderName::from_static(FILENAME_HEADER))
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            AppError::BadRequest(format!(
                "this upload has no {FILENAME_HEADER} header, so there is no way to tell what kind \
                 of file it is"
            ))
        })?;
    let name = raw.trim();
    let well_formed = !name.is_empty()
        && name.len() <= MAX_FILENAME_BYTES
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', ':'])
        && !name.chars().any(|c| c.is_ascii_control());
    well_formed
        .then(|| name.to_string())
        .ok_or_else(|| malformed(raw))
}

/// A short single-line field, bounded and free of anything that could not belong
/// in one.
///
/// Used for the statement balance, which is written into a journal — so a
/// newline or a control character is a correctness requirement rather than
/// tidiness. A LEADING `-` IS ALLOWED HERE and that is the point: a credit-card
/// statement balance is negative (`-3238.65` is the WP-11 contract's own
/// example), and refusing one would make the reconciliation unusable for exactly
/// the accounts people most want to reconcile.
fn plain_field(value: &str, what: &str) -> Result<String, AppError> {
    let trimmed = value.trim();
    let well_formed = !trimmed.is_empty()
        && trimmed.chars().count() <= MAX_FIELD_CHARS
        && !trimmed.chars().any(char::is_control);
    well_formed.then(|| trimmed.to_string()).ok_or_else(|| {
        AppError::BadRequest(format!(
            "{} is not a usable {what}: it must be a single line of at most {MAX_FIELD_CHARS} \
             characters",
            quoted(value)
        ))
    })
}

/// [`plain_field`], and additionally not option-shaped.
///
/// For the balance ACCOUNT, which becomes a positional query argument to
/// `hledger balance`. `Invocation` passes arguments as a `Vec<OsString>` with no
/// shell in sight, so there is nothing to quote — but a value beginning with `-`
/// is still read by hledger's own parser as a flag, and there is no `--`
/// terminator on a query. No account name begins with `-`, so refusing one costs
/// nothing.
fn argument_field(value: &str, what: &str) -> Result<String, AppError> {
    let field = plain_field(value, what)?;
    if field.starts_with('-') {
        return Err(AppError::BadRequest(format!(
            "{} is not a usable {what}: it may not begin with `-`, which hledger would read as an \
             option rather than a name",
            quoted(value)
        )));
    }
    Ok(field)
}

// ===========================================================================
// Security layers 2 and 3: resolution
// ===========================================================================

/// The `404` every resolution failure returns.
///
/// **Identical for every cause**, so the route cannot be used to tell "not
/// there" from "not a regular file" from "outside the root". It names the
/// caller's own handle and nothing else.
fn unresolved(what: &str, id: &str) -> AppError {
    AppError::NotFound(format!(
        "no {what} {} is available beside this journal",
        quoted(id)
    ))
}

/// The main journal file this server has open, or the `404` that says there is
/// none.
///
/// Answering the ordinary resolution failure — rather than a distinct "no
/// journal open" — keeps every route from distinguishing the two, the same rule
/// `rules_api::document` follows.
fn main_journal(state: &AppState, what: &str, id: &str) -> Result<PathBuf, AppError> {
    state
        .source_files()
        .into_iter()
        .next()
        .ok_or_else(|| unresolved(what, id))
}

/// The directory `include` (and therefore everything in this feature) is
/// confined to: the main journal file's own directory, canonicalized.
fn include_root(main: &Path) -> Result<PathBuf, AppError> {
    let dir = main.parent().unwrap_or_else(|| Path::new("."));
    std::fs::canonicalize(dir).map_err(|_| {
        AppError::Internal("this journal's own directory could not be resolved".to_string())
    })
}

/// Resolve a `journalId` to the file it names.
///
/// Layer 2, exactly as `Discovery::resolve` is for a rules id: the handle is
/// matched by **string equality against [`journals::targets`]**, which is a
/// projection of the files *this parse actually read*. The path handed back is
/// the one the parser recorded, not one built by joining the handle onto a root
/// — so a handle can only ever name a file the include guard already admitted.
///
/// A target the engine flagged as not writable (a symlink, a directory, a file
/// outside the include root) is refused with a `400` rather than a `404`: the
/// handle *did* resolve, and telling the user their journal is a symlink is
/// information they need.
fn resolve_journal(
    state: &AppState,
    root: &Path,
    id: &str,
) -> Result<(PathBuf, JournalTarget), AppError> {
    validate_relative_id(id, "journal id", None)?;
    let snapshot = state.snapshot();
    let target = journals::targets(&snapshot.journal)
        .into_iter()
        .find(|target| target.id == id)
        .ok_or_else(|| unresolved("journal", id))?;
    if !target.writable {
        return Err(AppError::BadRequest(format!(
            "{} cannot be imported into: an import target must be a regular file inside the \
             journal's own directory, not a symlink or a directory",
            quoted(id)
        )));
    }
    // The handle came out of `targets`, which derived it from this path; the
    // membership test below is what makes that round trip a checked fact rather
    // than an assumption.
    let path = root.join(id);
    snapshot
        .journal
        .source_files
        .iter()
        .any(|source| source == &path)
        .then_some((path, target))
        .ok_or_else(|| unresolved("journal", id))
}

/// Resolve a `csvPath` to the file it will be written to.
///
/// This is the one handle that names a file which need not exist, so it cannot
/// be resolved by membership in anything. Layer 3 does the work instead:
///
/// 1. the handle is already known to have only plain components (layer 1), so
///    joining it onto the root cannot escape lexically;
/// 2. its **parent directory** is canonicalized — which resolves every symlink in
///    the chain — and required to sit inside the canonical include root, so a
///    directory symlink pointing out of the tree is refused;
/// 3. an existing target must be a **regular file**: [`std::fs::symlink_metadata`]
///    does not follow links, so a symlink answers `false` on its own file type,
///    and a directory or a FIFO is refused for the same reason `rules::discover`
///    refuses one.
///
/// **Deviation from the WP-11 note, recorded deliberately.** The plan says to
/// confine with the shared `parse::confine`. That function is `pub(crate)` in
/// `ledgeline-core` and this crate cannot see it, and widening it is a change to
/// a crate this lane may not touch. What is here is `confine`'s own algorithm —
/// canonicalize, then test the prefix — applied to the parent, because
/// `confine`'s caller always holds a path that exists and this one deliberately
/// does not. If core ever exports it, this function should become a call to it.
fn resolve_destination(root: &Path, csv_path: &str) -> Result<PathBuf, AppError> {
    validate_relative_id(csv_path, "CSV destination", Some(".csv"))?;
    let candidate = root.join(csv_path);
    let (dir, name) = candidate
        .parent()
        .zip(candidate.file_name())
        .ok_or_else(|| unresolved("CSV destination", csv_path))?;

    let dir = std::fs::canonicalize(dir).map_err(|_| unresolved("CSV destination", csv_path))?;
    if !dir.starts_with(root) {
        return Err(unresolved("CSV destination", csv_path));
    }
    let destination = dir.join(name);
    match std::fs::symlink_metadata(&destination) {
        // Nothing there yet: exactly the common case for a fresh statement.
        Err(_) => Ok(destination),
        Ok(meta) if meta.file_type().is_file() => Ok(destination),
        Ok(_) => Err(AppError::BadRequest(format!(
            "{} cannot be written: a CSV destination must be a regular file, not a symlink, a \
             directory or a device",
            quoted(csv_path)
        ))),
    }
}

/// Resolve a `rulesId` through the discovery scan — the only id → rules-file
/// resolution in this codebase, by exact string equality against a set scanned
/// in this request.
fn resolve_rules(discovery: &Discovery, id: &str) -> Result<PathBuf, AppError> {
    validate_relative_id(id, "rules file id", Some(".rules"))?;
    discovery
        .resolve(id)
        .map(|found| found.path().as_path().to_path_buf())
        .ok_or_else(|| unresolved("rules file", id))
}

/// Resolve a `stageId` through the staging area — see [`crate::stage`].
fn resolve_stage(state: &AppState, raw: &str) -> Result<std::sync::Arc<Stage>, AppError> {
    StageId::parse(raw)
        .and_then(|id| state.stages().get(&id))
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "{} is not a staged upload this server is holding; drop the file again",
                quoted(raw)
            ))
        })
}

// ===========================================================================
// Security layer 5: no absolute path is ever echoed
// ===========================================================================

/// Rewrites the absolute paths *we* handed a subprocess back into the relative
/// handles the client already has.
///
/// hledger and git both name files in their diagnostics — `imported 3 new
/// transactions from bank.csv to /Users/me/finances/main.journal` is verbatim
/// real output — and those diagnostics are the single most useful thing on the
/// screen when something goes wrong, so withholding them is not an option. The
/// same trade `git.rs` makes with its `scrub`: pass the words through, redact the
/// prefixes we know.
///
/// Longest-first, so `/root/import/bank.csv` becomes `import/bank.csv` rather
/// than being half-rewritten by the root's own entry.
#[derive(Debug, Default, Clone)]
struct Redactor {
    swaps: Vec<(String, String)>,
}

impl Redactor {
    /// Replace every occurrence of `path` with `handle`.
    ///
    /// Both the path as given and its canonical spelling are registered, because
    /// they differ routinely — on macOS `/tmp` is a symlink to `/private/tmp`,
    /// and a subprocess may report either.
    fn hide(mut self, path: &Path, handle: &str) -> Self {
        let mut spellings = vec![path.to_string_lossy().into_owned()];
        if let Ok(canonical) = std::fs::canonicalize(path) {
            spellings.push(canonical.to_string_lossy().into_owned());
        }
        for spelling in spellings {
            if !spelling.is_empty() && !self.swaps.iter().any(|(from, _)| *from == spelling) {
                self.swaps.push((spelling, handle.to_string()));
            }
        }
        self
    }

    /// Replace a DIRECTORY prefix: `dir/x` becomes `x`, and a bare mention of the
    /// directory becomes `.`. Same two substitutions `git.rs::scrub` makes.
    fn hide_prefix(mut self, dir: &Path) -> Self {
        let spelling = dir.to_string_lossy().into_owned();
        let canonical = std::fs::canonicalize(dir)
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|_| spelling.clone());
        for prefix in [spelling, canonical] {
            if prefix.is_empty() {
                continue;
            }
            self.swaps.push((format!("{prefix}/"), String::new()));
            self.swaps.push((prefix, ".".to_string()));
        }
        self
    }

    /// `text` with every known path rewritten.
    fn apply(&self, text: &str) -> String {
        let mut swaps: Vec<&(String, String)> = self.swaps.iter().collect();
        swaps.sort_by_key(|(from, _)| std::cmp::Reverse(from.len()));
        swaps.iter().fold(text.to_string(), |redacted, (from, to)| {
            redacted.replace(from.as_str(), to.as_str())
        })
    }
}

// ===========================================================================
// hledger invocations
// ===========================================================================

/// The `501` a missing or too-old hledger produces.
///
/// Not a `500`: nothing broke. The server genuinely cannot perform this
/// operation, and `native.ts` maps a `501` to `NativeApiUnavailableError`, whose
/// `.message` is exactly the actionable banner the WP-11 definition of done asks
/// for ("Missing / too-old hledger produces an actionable banner with a
/// path-setting control, never a stack trace").
///
/// It reuses [`AppError::EditingDisabled`] because that is this crate's `501`
/// and a new variant would mean editing `error.rs`, which is another lane's
/// file. The variant is named for the condition it was introduced for; the
/// status and the SPA's handling are what is being reused.
fn hledger_unavailable(error: &HledgerError) -> AppError {
    AppError::EditingDisabled(format!(
        "{error}. Ledgeline needs hledger {} or newer to import; set its path in Preferences if \
         it is installed somewhere unusual.",
        crate::hledger::MIN_HLEDGER
    ))
}

/// A resolved hledger, or the `501`.
fn resolve_hledger() -> Result<Hledger, AppError> {
    Hledger::resolve(&prefs::load()).map_err(|error| hledger_unavailable(&error))
}

/// Run an invocation, mapping a transport failure (spawn, timeout) to a `500`.
///
/// A NON-ZERO EXIT IS NOT AN ERROR HERE. hledger exits non-zero for a failed
/// check, a rules error and a balance assertion, all of which are answers this
/// module reports rather than failures it hides.
fn run(invocation: Invocation, what: &str) -> Result<Output, AppError> {
    invocation
        .run()
        .map_err(|error| AppError::Internal(format!("could not {what}: {error}")))
}

/// `hledger [--alias=…]… -f JOURNAL import [--dry-run] --rules RULES CSV`.
///
/// Every path is absolute, which is also what makes them safe as positional
/// arguments: an absolute path begins with `/` and can never be read as an
/// option, so no `--` terminator is needed to protect a statement named `-f`.
///
/// `--rules`, not `--rules-file`: the flag was renamed in hledger 1.40, which is
/// why [`MIN_HLEDGER`](crate::hledger::MIN_HLEDGER) is 1.40.
///
/// # `--dry-run` is a parameter, and that is the point
///
/// The preview and the write share **one** argv builder, so there is no way for
/// them to be given different aliases. That matters more than it reads: a
/// preview that showed `assets:morganstanley:pw-roth-ira` while the commit wrote
/// `PW Roth IRA - 3077` would be a lie told immediately before the only
/// irreversible step on the screen. Making the two agree by construction is
/// stronger than a test asserting that they do — though
/// `import_endpoints.rs::a_dry_run_and_a_commit_agree_on_aliased_accounts` also
/// asserts it, against the real binary.
fn import_invocation(
    hledger: &Hledger,
    journal: &Path,
    rules: &Path,
    csv: &Path,
    aliases: &[String],
    dry_run: bool,
) -> Invocation {
    let invocation = hledger
        .invoke(alias_flags(aliases))
        .args(["-f".as_ref(), journal.as_os_str()])
        .arg("import");
    let invocation = if dry_run {
        invocation.arg("--dry-run")
    } else {
        invocation
    };
    invocation
        .arg("--rules")
        .arg(rules)
        .arg(csv)
        .timeout(IMPORT_TIMEOUT)
}

/// The journal's aliases as hledger options, one `OsString` each.
///
/// # Why the joined `--alias=VALUE` spelling
///
/// The two-token form works — verified, even for a value beginning with `-` —
/// but one token cannot be misread as a flag boundary by any option parser,
/// present or future. It is the same instinct as the absolute-path rule above:
/// prefer the spelling that has no ambiguity to resolve.
///
/// The values are user text on their way to `argv`, so they are `OsString`s
/// handed to `Command::args` and **never** a shell string; there is no shell
/// anywhere in this codebase to quote for. `ledgeline_core::aliases::forward`
/// has already refused any alias carrying a control character or exceeding its
/// length caps, and bounded how many there may be.
fn alias_flags(aliases: &[String]) -> Vec<std::ffi::OsString> {
    aliases
        .iter()
        .map(|alias| std::ffi::OsString::from(format!("--alias={alias}")))
        .collect()
}

/// How many transactions `text` holds, as our own parser reads it.
///
/// This is what turns "hledger's stdout is re-parseable journal text" from a
/// claim into something the count depends on. `None` when it does not parse,
/// which is the caller's cue to fall back to hledger's own status line.
fn count_transactions(text: &str) -> Option<usize> {
    ledgeline_core::parse_journal(text, "proposed")
        .ok()
        .map(|journal| journal.transactions.len())
}

/// The integer in a status line like `would import 3 new transactions from …`.
///
/// The fallback for [`count_transactions`], and only the fallback: scraping a
/// human-readable line is precisely what the stdout/stderr split exists to avoid,
/// but reporting `0` for a successful import because our parser tripped over an
/// entry would be worse.
fn reported_count(stderr: &str, verb: &str) -> Option<usize> {
    stderr
        .lines()
        .find_map(|line| line.trim().strip_prefix(verb))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|token| token.parse().ok())
}

/// Verify a statement balance over **journal + proposed entries**, as one
/// journal.
///
/// # Why this is not two `-f` flags
///
/// Balance ASSERTIONS do not aggregate across multiple `-f` files. This is a
/// silent wrong answer, not an error — fact 3 in `plans/11-enhanced-import.md`,
/// re-verified against hledger 1.52 while building this:
///
/// ```text
/// $ hledger -f main.journal -f new.journal balance assets:bank:checking
/// Balance assertion failed … the asserted balance is: $930.00
///                            but the calculated balance is: $-20.00
///
/// $ printf 'include /abs/main.journal\n' | cat - new.journal | hledger -f- balance …
///              $930.00  assets:bank:checking
/// ```
///
/// The second file's assertions never saw the first file's balances. So
/// verification hands hledger **one** journal on stdin, through
/// [`Invocation::stdin`], and there is exactly one `-f` on the command line.
/// `tests/import_endpoints.rs` asserts the difference directly.
///
/// # Why an `include` line rather than the journal's bytes
///
/// The plan spells the concatenation `cat journal proposed`. Taken literally
/// that breaks any journal with an `include` in it: a journal read from stdin
/// has no directory of its own, so hledger resolves relative includes against
/// the **process's working directory** — verified, and it fails with
/// `No files were matched by: sub/inc.journal`. `Invocation` deliberately offers
/// no way to set a working directory (a per-call `cwd` is one more thing a
/// subprocess invariant could be built around and then forgotten).
///
/// So the first line is `include <ABSOLUTE PATH>`, which is the plan's own
/// second sanctioned form ("or use a temp wrapper of `include` lines. Both were
/// verified to give the correct combined balance") delivered through the pipe
/// instead of a temp file. It is still one journal and still one `-f`, which is
/// the property fact 3 is about, and it is strictly more faithful: `Y`,
/// `apply account` and `commodity` scoping stay per-file exactly as they are when
/// hledger reads the journal normally.
fn verify_balance(
    hledger: &Hledger,
    journal: &Path,
    proposed: &str,
    account: &str,
) -> Result<Option<String>, AppError> {
    let output = run(
        hledger
            .invoke(["-f", "-", "balance"])
            .arg(account)
            .args(["--no-total", "--flat", "-O", "csv"])
            .stdin(concatenated(journal, proposed).into_bytes())
            .timeout(BALANCE_TIMEOUT),
        "compute the combined balance",
    )?;
    // A FAILED command and an empty result mean different things, and collapsing
    // them is how a reconciliation reports a confident `0.00` for a balance it
    // never computed. hledger exits non-zero here when an assertion inside the
    // combined journal fails — which is real information, not an absence — so
    // that case answers `None` ("not known"), while a query that simply matched
    // no account answers `Some("0")`, which is that account's true balance.
    Ok(output
        .success()
        .then(|| balance_from_csv(&output.stdout_lossy()).unwrap_or_else(|| "0".to_string())))
}

/// The single journal handed to hledger on stdin: the real journal by absolute
/// `include`, then the proposed entries. See [`verify_balance`].
fn concatenated(journal: &Path, proposed: &str) -> String {
    format!("include {}\n\n{proposed}\n", journal.display())
}

/// The balance out of `hledger balance -O csv` output.
///
/// CSV rather than the human table so nothing here is scraping prose, and
/// `--no-total` so the only rows are accounts. `None` when the query matched no
/// account at all, which is a real answer — "that account does not exist in the
/// combined journal" — and not the same as a balance of zero.
fn balance_from_csv(csv: &str) -> Option<String> {
    csv.lines()
        .skip(1)
        .filter(|line| !line.trim().is_empty())
        .last()
        .and_then(|line| line.rsplit_once(','))
        .map(|(_, balance)| balance.trim().trim_matches('"').to_string())
}

/// A money string split into the **commodity that surrounds it** and the
/// numeral inside it: `$2,945.05`, `-1234.5`, `2945.05 USD`, `$-5.00`.
///
/// Deliberately narrow, and it **declines loudly rather than guessing**.
/// Anything it cannot read — a multi-commodity balance, a European decimal
/// comma we were not told about — yields `None`, which surfaces as
/// `difference: null` in a reconciliation and as a **refusal** to write a
/// balance assertion.
///
/// # Why the commodity is a field and not something to strip
///
/// An amount with no commodity is not an amount without a commodity: it is an
/// amount in the *empty* commodity. `assets:bank:checking  0 = 2949.80` asserts
/// 2949.80 of nothing, hledger computes 0 for it, and the assertion fails on
/// every later `hledger check` — fact 4 of `plans/11-enhanced-import.md`,
/// arriving in our own output rather than in a user's rules file. So
/// [`Amount::has_commodity`] is a question this module has to be able to ask,
/// and [`Amount::wrap`] is how the commodity hledger itself reported is carried
/// onto the number the user typed. See [`assertion_lines`].
///
/// # The shape, and the trap it is shaped around
///
/// One numeric run, with a commodity that may lead or trail. The run is bounded
/// by the FIRST non-numeric character after it, and whatever follows may not
/// contain another digit, a `.` or a `,`.
///
/// That last condition is the whole reason this is not four lines. hledger
/// reports a two-commodity balance as `$10.00, 3 AAPL`, and stripping `,` as a
/// thousands separator turns it into `10.003` — a plausible, confidently wrong
/// number for an account holding two commodities, reported as if it were the
/// truth. A unit test pins it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Amount {
    /// Everything before the numeral with the sign removed: `$`, `€`, or empty.
    prefix: String,
    /// Everything after the numeral: ` USD`, or empty.
    suffix: String,
    negative: bool,
    /// The numeral as written, digit-group separators included.
    number: String,
}

impl Amount {
    /// The value, exactly. `None` only when the numeral overflows the engine's
    /// exact decimal.
    fn value(&self) -> Option<Dec> {
        // Group separators go only now, once the whole run is known to be one
        // number in the one notation `is_anglo_numeral` accepts.
        let digits: String = self.number.chars().filter(|c| *c != ',').collect();
        let value = Dec::parse(&digits, '.').ok()?;
        if self.negative {
            value.neg().ok()
        } else {
            Some(value)
        }
    }

    /// Does this amount name a commodity at all? `false` is the trap.
    fn has_commodity(&self) -> bool {
        !self.prefix.is_empty() || !self.suffix.is_empty()
    }

    /// `text` wrapped in THIS amount's commodity: `$` around `2945.05`.
    fn wrap(&self, text: &str) -> String {
        format!("{}{text}{}", self.prefix, self.suffix)
    }
}

/// One money string as an [`Amount`], or `None` when it is not one amount.
fn split_amount(text: &str) -> Option<Amount> {
    let text = text.trim();
    let first_digit = text.find(|c: char| c.is_ascii_digit())?;
    let (head, rest) = text.split_at(first_digit);
    // Everything before the first digit is the commodity and, possibly, the
    // sign. A `.` or `,` in there means this is not one amount.
    if head.contains(['.', ',']) || head.chars().filter(|c| *c == '-' || *c == '+').count() > 1 {
        return None;
    }

    let numeric = rest.len() - rest.trim_start_matches(is_numeric).len();
    let (number, tail) = rest.split_at(numeric);
    // A trailing commodity (` USD`) is fine; a second amount is not.
    if tail.chars().any(is_numeric) || !is_anglo_numeral(number) {
        return None;
    }
    Some(Amount {
        prefix: head.replace(['-', '+'], "").trim().to_string(),
        suffix: tail.to_string(),
        negative: head.contains('-'),
        number: number.to_string(),
    })
}

/// Part of a decimal numeral: a digit, a group separator, or a decimal mark.
fn is_numeric(c: char) -> bool {
    c.is_ascii_digit() || c == ',' || c == '.'
}

/// Is `number` a `.`-as-decimal-mark numeral with `,` thousands groups?
///
/// Two conditions, and each rules out one European spelling that would otherwise
/// be read as a different number entirely:
///
/// * **at most one `.`** — `1.234.567,89` is one and a half million euros, and
///   without this it reads as `1.23456789`;
/// * **every `,` introduces exactly three digits** — `1,23` is one euro
///   twenty-three, and without this it reads as one hundred and twenty-three.
///
/// Ledgeline does not know which notation a bank used, and the rules file's
/// `decimal-mark` is about the *statement*, not about what the user typed into
/// the balance field. Declining is the only honest answer.
fn is_anglo_numeral(number: &str) -> bool {
    let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
    !fraction.contains(['.', ','])
        && whole
            .split(',')
            .enumerate()
            .all(|(at, group)| (at == 0 || group.len() == 3) && !group.is_empty())
}

/// `value` as decimal text at exactly `places` fractional digits, padding with
/// zeros when it has fewer.
///
/// `Dec` has no `Display` — it is a wire type whose mantissa and scale are
/// carried verbatim — so the one place that needs human-readable money renders
/// it here.
///
/// The reconciliation renders both of its sides at one scale, so `2949.8` typed
/// into the form and `$2949.80` computed by hledger do not sit beside each other
/// looking like two different numbers. Never truncates: `places` below the
/// value's own scale keeps the value's, because dropping a digit to make two
/// numbers look alike is how a 5-cent gap disappears from a screen.
fn render_money_at(value: Dec, places: u32) -> String {
    let pad = usize::try_from(places.saturating_sub(value.places)).unwrap_or(0);
    let places = usize::try_from(value.places.max(places)).unwrap_or(0);
    let digits = format!("{}{}", value.mantissa.unsigned_abs(), "0".repeat(pad));
    let padded = if digits.len() <= places {
        format!("{}{digits}", "0".repeat(places + 1 - digits.len()))
    } else {
        digits
    };
    let (whole, fraction) = padded.split_at(padded.len() - places);
    let sign = if value.mantissa < 0 { "-" } else { "" };
    if places == 0 {
        format!("{sign}{whole}")
    } else {
        format!("{sign}{whole}.{fraction}")
    }
}

/// The reconciliation the UI shows: what the user said, what hledger computed,
/// and the gap — **all three in one representation**.
///
/// `computed: None` means hledger could not give us a number at all. That
/// reports an EMPTY string rather than `"0"` — a zero is a real balance and this
/// is the absence of one — and it never claims a match.
///
/// # Why one representation is a correctness requirement, not tidiness
///
/// These strings are rendered side by side ("your statement says X; the journal
/// plus these transactions computes to Y"). They used to be formatted
/// differently on each side: the user typed `2949.80`, hledger answered
/// `$2949.80`, and a screen showing both read as a mismatch at a glance while
/// `matches` said otherwise. A user who trusts their eyes over the badge then
/// re-types a balance that was already right.
///
/// So the commodity **hledger reported for the account** wins, is applied to
/// every field, and both numerals are rendered at the same scale. hledger's
/// rather than the user's because it is the journal's own, and it is the one a
/// balance assertion would be written in ([`assertion_lines`]) — echoing `USD`
/// back at a journal kept in `$` would be a prettier way of saying something
/// untrue. The user's own commodity is the fallback for a computed balance that
/// carries none, and a bare pair stays bare, which is right for a journal that
/// genuinely has no commodity.
///
/// When either side is not one amount — a multi-commodity balance, most likely —
/// both are reported **verbatim**, `difference` is `null` and no match is
/// claimed. There is no single commodity to render them in and no one number to
/// be off by, and inventing either is the wrong-answer class this whole function
/// is shaped around.
fn reconcile(statement: &str, computed: Option<String>) -> WireBalance {
    let verbatim = |computed: String| WireBalance {
        statement: statement.to_string(),
        computed,
        matches: false,
        difference: None,
    };
    let Some(computed) = computed else {
        return verbatim(String::new());
    };
    let (Some(typed), Some(reported)) = (split_amount(statement), split_amount(&computed)) else {
        return verbatim(computed);
    };
    let (Some(typed_value), Some(reported_value)) = (typed.value(), reported.value()) else {
        return verbatim(computed);
    };
    let Ok(difference) = typed_value.sub(reported_value) else {
        return verbatim(computed);
    };

    let commodity = if reported.has_commodity() {
        &reported
    } else {
        &typed
    };
    let places = typed_value.places.max(reported_value.places);
    WireBalance {
        statement: commodity.wrap(&render_money_at(typed_value, places)),
        computed: commodity.wrap(&render_money_at(reported_value, places)),
        matches: difference.is_zero(),
        difference: Some(commodity.wrap(&render_money_at(difference, places))),
    }
}

// ===========================================================================
// git
// ===========================================================================

/// Whether the git safety net is switched on: `None` means "commit when a repo
/// is present", which is the default posture `git.rs` implements.
fn autocommit_enabled(prefs: &Prefs) -> bool {
    prefs.git_autocommit.unwrap_or(true)
}

/// Which of `targets` git reports as MODIFIED, repository by repository.
///
/// Untracked never blocks — a brand-new CSV is expected to be untracked. A
/// target outside any repository, or a git that cannot be run at all, contributes
/// nothing: there is no safety net to lose, and refusing the import over its
/// absence would be worse than not having it.
///
/// Returns the caller's own relative handles rather than git's repo-relative
/// paths, so the UI names the files with the same strings the form does.
fn blocked_by_git(targets: &[(&Path, &str)]) -> Vec<String> {
    targets
        .iter()
        .filter(|(path, _)| !git_status(path).dirty.is_empty())
        .map(|(_, handle)| (*handle).to_string())
        .collect()
}

/// What git thinks of one target, degrading uniformly to
/// [`GitStatus::unavailable`].
///
/// "No repository here", "git is not installed" and "git could not answer" mean
/// the same thing to this module — there is no safety net for this file — and
/// collapsing them is what keeps a missing git from *blocking* an import rather
/// than merely not protecting it. `dirty` is git.rs's own precomputed
/// [`FileState::Modified`] subset, which is the only state that blocks.
fn git_status(path: &Path) -> GitStatus {
    Repo::discover(path).map_or_else(GitStatus::unavailable, |repo| {
        repo.status(&[path])
            .unwrap_or_else(|_| GitStatus::unavailable())
    })
}

/// Commit exactly the paths this import wrote, grouped by repository.
///
/// Each target is resolved on its own, because the CSV destination and the
/// journal may live in different repositories or one may be outside version
/// control entirely — `git.rs` makes [`Repo`] `Eq` for exactly this grouping. A
/// target with no repository is reported as skipped, not as a failure.
fn commit_targets(targets: &[(&Path, &str)], message: &str, redactor: &Redactor) -> WireGitResult {
    let (in_repo, outside): (Vec<_>, Vec<_>) = targets
        .iter()
        .map(|(path, handle)| (Repo::discover(path), *path, *handle))
        .partition(|(repo, _, _)| repo.is_some());

    let mut committed = Vec::new();
    let mut skipped: Vec<String> = outside
        .iter()
        .map(|(_, _, handle)| (*handle).to_string())
        .collect();
    let mut message_out = None;

    // Group by repository, preserving first-seen order so the result is stable.
    let repos: Vec<Repo> = in_repo.iter().filter_map(|(repo, _, _)| repo.clone()).fold(
        Vec::new(),
        |mut seen, repo| {
            if !seen.contains(&repo) {
                seen.push(repo);
            }
            seen
        },
    );
    for repo in &repos {
        let group: Vec<(&Path, &str)> = in_repo
            .iter()
            .filter(|(candidate, _, _)| candidate.as_ref() == Some(repo))
            .map(|(_, path, handle)| (*path, *handle))
            .collect();
        let paths: Vec<&Path> = group.iter().map(|(path, _)| *path).collect();
        match repo.commit(&paths, message) {
            Ok(()) => committed.extend(group.iter().map(|(_, handle)| (*handle).to_string())),
            Err(error) => {
                skipped.extend(group.iter().map(|(_, handle)| (*handle).to_string()));
                // The FIRST failure's words, not the last: a second repository
                // failing for a different reason is rarer than the same hook
                // rejecting both, and the first one is the one the user acts on.
                //
                // Redacted against this repository's own toplevel as well as the
                // journal's paths. `git.rs` already scrubs the toplevel out of
                // the errors it builds from git's output, and `Repo::toplevel`
                // exists precisely because that path "must never reach a
                // user-facing string" — so layer 5 is asserted here rather than
                // inherited.
                let scrubbed = redactor
                    .clone()
                    .hide_prefix(repo.toplevel())
                    .apply(&error.to_string());
                message_out.get_or_insert(scrubbed);
            }
        }
    }

    WireGitResult {
        committed: !committed.is_empty(),
        paths: committed,
        skipped,
        message: message_out,
    }
}

// ===========================================================================
// Handlers
// ===========================================================================

/// `Cache-Control: no-store`, no `ETag` — the same posture as the rules routes.
/// None of this is derived from the journal snapshot, and the `ETag` is one
/// per-journal generation counter shared by every read endpoint, so there is no
/// third option: tagging these would either invalidate the SPA's cached
/// `/transactions` body or hand out a stale one under a fresh tag.
fn no_store<T: Serialize>(body: T) -> Response {
    const NO_STORE: (HeaderName, HeaderValue) =
        (header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    ([NO_STORE], Json(body)).into_response()
}

/// `GET /api/import/capabilities` — hledger, formats, target journals, git.
pub(crate) async fn capabilities(State(state): State<AppState>) -> Result<Response, AppError> {
    let editable = state.editing_enabled();
    let snapshot = state.snapshot();
    // Through `compute`: resolving hledger spawns a process and probing git
    // spawns another, which is exactly the blocking work the semaphore and the
    // blocking pool exist for.
    let aliases = state.clone();
    let Json(body) = compute(move || {
        let prefs = prefs::load();
        let resolved = Hledger::resolve(&prefs);
        Ok(WireCapabilities {
            hledger: WireHledger::from(&resolved),
            formats: convert_formats(),
            journals: journals::targets(&snapshot.journal)
                .iter()
                .map(WireJournal::from)
                .collect(),
            git: WireGit {
                available: crate::git::git_available(),
                autocommit: autocommit_enabled(&prefs),
            },
            // The same builder the alias editor uses, so the two screens cannot
            // disagree about what is in force — which would make the whole
            // point of showing it moot.
            aliases: crate::alias_api::capability_aliases(&aliases),
            editable,
        })
    })
    .await?;
    Ok(no_store(body))
}

/// Every format the New Transactions tab accepts, in the order the UI lists
/// them. Spelled from the engine's own names so the two cannot drift.
fn convert_formats() -> Vec<&'static str> {
    [
        SourceFormat::Csv,
        SourceFormat::Tsv,
        SourceFormat::Ssv,
        SourceFormat::Ofx,
        SourceFormat::Qfx,
        SourceFormat::Xls,
        SourceFormat::Xlsx,
        SourceFormat::Xlsm,
        SourceFormat::Xlsb,
        SourceFormat::Ods,
    ]
    .into_iter()
    .map(SourceFormat::as_str)
    .collect()
}

/// `POST /api/import/stage` — raw bytes plus `X-Ledgeline-Filename`.
///
/// The one upload primitive in the whole API. Size is capped by an axum
/// `DefaultBodyLimit` on **this route alone** (see `lib.rs`); the check repeated
/// below is not redundant — it is what bounds a caller that reaches this handler
/// some other way, and it is the layer that produces a sentence rather than a
/// bare `413`.
pub(crate) async fn stage(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, AppError> {
    let name = bare_filename(&headers)?;
    if body.len() > stage::MAX_UPLOAD_BYTES {
        return Err(AppError::BadRequest(format!(
            "this file is larger than the {} MiB import limit",
            stage::MAX_UPLOAD_BYTES / (1024 * 1024)
        )));
    }
    let Json(body) = compute(move || stage_upload(&state, &name, &body)).await?;
    Ok(no_store(body))
}

/// The whole of `stage`, synchronously, on the blocking pool.
fn stage_upload(state: &AppState, name: &str, bytes: &[u8]) -> Result<WireStage, AppError> {
    let format = convert::detect(name, bytes).map_err(convert_error)?;
    let tabular = convert::convert(format, bytes).map_err(convert_error)?;
    let csv = convert::to_csv(&tabular);

    let (id, staged) = state.stages().put(&csv, format).map_err(|error| {
        AppError::Internal(format!("could not stage this upload: {}", error.kind()))
    })?;

    // Candidate scoring needs the journal's own directory to scan for rules
    // files. Without a journal there is nothing to score against — which is not
    // an error, just an empty list and a default derived from the upload's name.
    let main = state.source_files().into_iter().next();
    let aliases = aliases::arguments(&state.snapshot().journal);
    let candidates = match (&main, resolve_hledger()) {
        (Some(main), Ok(hledger)) => rank_candidates(&hledger, main, &staged, &tabular, &aliases),
        _ => Vec::new(),
    };

    let defaults = defaults_for(state, name, candidates.first().map(|c| c.id.as_str()));
    // Materialise under the default destination now, so a dry-run that accepts
    // the defaults already has the right dedup state beside it; a dry-run that
    // changes the destination re-materialises under the new name.
    //
    // BEST EFFORT, deliberately. This is a warm-up, not the thing the dry-run
    // depends on — `run_dry_run` materialises unconditionally and propagates its
    // own failure — so a default the user is about to change, naming a directory
    // that does not exist yet, must not fail the upload they just made.
    if let Some(root) = main.as_deref().map(include_root).transpose()?
        && let Ok(destination) = resolve_destination(&root, &defaults.csv_path)
        && let Some((dir, file)) = destination.parent().zip(file_name(&destination))
    {
        let _ = staged.materialize(RUN_WITH_LATEST, &file, Some(dir));
    }

    Ok(WireStage {
        stage_id: id.as_str().to_string(),
        format: staged.format().as_str(),
        preview: preview_of(&tabular),
        statement: tabular.statement.as_ref().map(WireStatement::from),
        notes: tabular.notes.iter().map(WireNote::from).collect(),
        candidates,
        defaults,
    })
}

/// A conversion failure as an HTTP error.
///
/// Every variant is a `400`: each describes the file the caller sent, and none
/// of them is something this server did wrong. `PdfNotSupported` reaches the user
/// as its own sentence rather than as a generic parse failure, which is the whole
/// reason `convert::detect` returns a `Result` instead of an `Option`.
fn convert_error(error: ConvertError) -> AppError {
    AppError::BadRequest(error.to_string())
}

/// A path's final component as a `String`.
fn file_name(path: &Path) -> Option<String> {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
}

/// The preview a stage response carries: bounded in rows, columns and cell size.
fn preview_of(tabular: &Tabular) -> WirePreview {
    WirePreview {
        header: tabular.header.as_ref().map(|row| clip_row(row)),
        rows: tabular
            .rows
            .iter()
            .take(PREVIEW_ROWS)
            .map(|row| clip_row(row))
            .collect(),
        row_count: tabular.rows.len(),
        truncated: tabular.truncated,
    }
}

/// One preview row, clipped in width and per cell.
fn clip_row(row: &[String]) -> Vec<String> {
    row.iter()
        .take(PREVIEW_COLUMNS)
        .map(|cell| cell.chars().take(PREVIEW_CELL_CHARS).collect())
        .collect()
}

/// Score every plausible rules file against the staged data, best first.
///
/// Two stages, as `matching.rs` specifies. Stage 1 is pure and rejects the
/// obviously-wrong so stage 2 is cheap; stage 2 runs
/// `hledger print -f DATA --rules R -O json` on at most
/// [`MAX_SCORED_CANDIDATES`](matching::MAX_SCORED_CANDIDATES) survivors and
/// derives the signals from the JSON — `-O json` precisely so no human-readable
/// output is ever regex-scraped.
///
/// The pre-rank that decides *which* eight survivors get a subprocess is by
/// modification time, the same tie-break [`matching::rank`] applies afterwards:
/// a user with `checking-2024.csv.rules` and `checking-2025.csv.rules` is still
/// importing into the one they touched most recently.
fn rank_candidates(
    hledger: &Hledger,
    main: &Path,
    staged: &Stage,
    data: &Tabular,
    aliases: &[String],
) -> Vec<WireCandidate> {
    let discovery = rules::discover(main);
    let mut survivors: Vec<(usize, matching::PrefilterPass)> = discovery
        .files
        .iter()
        .take(MAX_PREFILTERED)
        .enumerate()
        .filter(|(_, found)| found.parsed)
        .filter_map(|(at, found)| {
            let text = std::fs::read_to_string(found.path().as_path()).ok()?;
            let pass = matching::prefilter(&RulesDoc::parse(&text), data)?;
            Some((at, pass))
        })
        .collect();
    survivors.sort_by(|(a, _), (b, _)| {
        discovery.files[*b]
            .modified
            .cmp(&discovery.files[*a].modified)
            .then_with(|| discovery.files[*a].id.cmp(&discovery.files[*b].id))
    });

    let statement_currency = data
        .statement
        .as_ref()
        .and_then(|meta| meta.currency.clone());
    let mut rankings: Vec<(Ranking, CandidateExtras)> = survivors
        .into_iter()
        .take(matching::MAX_SCORED_CANDIDATES)
        .filter_map(|(at, pass)| {
            let found = &discovery.files[at];
            let json = print_json(hledger, &staged.data(), found.path().as_path(), aliases)?;
            let expected = pass
                .expected_commodity
                .clone()
                .or_else(|| statement_currency.clone());
            let signals = matching::signals_from_hledger_json(&json, expected.as_deref())
                .ok()?
                .with_prefilter(&pass);
            Some((
                Ranking {
                    candidate: Candidate {
                        id: found.id.clone(),
                        label: found.label.clone(),
                        score: matching::score(&signals),
                        signals,
                    },
                    modified: found.modified,
                },
                CandidateExtras {
                    sample: sample_from_json(&json),
                    account1: found.account1.clone(),
                    account2: found.account2.clone(),
                },
            ))
        })
        .collect();

    // `rank` sorts `[Ranking]`, so everything the wire carries that
    // `matching::Candidate` does not is held alongside and re-joined by id
    // afterwards rather than being threaded through the ranking type.
    let mut ordered: Vec<Ranking> = rankings
        .iter()
        .map(|(ranking, _)| ranking.clone())
        .collect();
    matching::rank(&mut ordered);
    ordered
        .into_iter()
        .map(|ranking| {
            let extras = rankings
                .iter_mut()
                .find(|(other, _)| other.candidate.id == ranking.candidate.id)
                .map(|(_, extras)| std::mem::take(extras))
                .unwrap_or_default();
            WireCandidate {
                id: ranking.candidate.id,
                label: ranking.candidate.label,
                score: score_value(ranking.candidate.score),
                signals: WireSignals::from(&ranking.candidate.signals),
                sample: extras.sample,
                account1: extras.account1,
                account2: extras.account2,
            }
        })
        .collect()
}

/// What the wire carries about a candidate that `matching::Candidate` does not:
/// the sample transactions, and the rules file's own default accounts.
#[derive(Default)]
struct CandidateExtras {
    sample: Vec<WireProposed>,
    account1: Option<String>,
    account2: Option<String>,
}

/// A [`Score`] as the number the wire carries.
fn score_value(score: Score) -> f32 {
    score.value()
}

/// `hledger [--alias=…]… print -f DATA --rules RULES -O json`, parsed.
///
/// `None` on any failure — a rules file hledger refuses simply does not become a
/// candidate, which is the correct outcome and not something to report: the user
/// asked which of their rules files fits, not for a list of the ones that do not
/// parse.
///
/// The aliases go on here too, and the rule that puts them here is worth stating
/// once: **`--alias` goes on every invocation that reads the CSV, and on no
/// other.** This one reads the CSV, so a candidate's sample accounts are the
/// accounts the dry-run will propose. Without that the card would advertise
/// `PW Roth IRA - 3077` and the preview two sections below it would say
/// `assets:morganstanley:pw-roth-ira`, and the user would have to work out which
/// of the two was lying.
///
/// The invocations that do *not* get them are the balance ones
/// ([`verify_balance`], [`preflight_assertion`]). Those read a journal, and a
/// journal's accounts are already the names it was written with — hledger
/// applies the journal's own `alias` directives when it reads them, exactly as
/// it always has. Adding `--alias` there would apply the mapping a second time,
/// and a regex alias broad enough to match its own output would then rewrite an
/// account that was already correct.
fn print_json(hledger: &Hledger, data: &Path, rules: &Path, aliases: &[String]) -> Option<Value> {
    let output = hledger
        .invoke(alias_flags(aliases))
        .args(["print".as_ref(), "-f".as_ref(), data.as_os_str()])
        .arg("--rules")
        .arg(rules)
        .args(["-O", "json"])
        .timeout(IMPORT_TIMEOUT)
        .run()
        .ok()?;
    output
        .success()
        .then(|| serde_json::from_str(&output.stdout_lossy()).ok())
        .flatten()
}

/// The first couple of transactions out of `hledger print -O json`, flattened
/// for display.
fn sample_from_json(json: &Value) -> Vec<WireProposed> {
    /// Enough to recognise your own statement, few enough to render in a card.
    const SAMPLE_TXNS: usize = 2;
    json.as_array()
        .into_iter()
        .flatten()
        .take(SAMPLE_TXNS)
        .map(|txn| WireProposed {
            date: field(txn, "tdate"),
            description: field(txn, "tdescription"),
            postings: txn
                .get("tpostings")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|posting| field(posting, "paccount"))
                .collect(),
        })
        .collect()
}

/// One string field of an hledger JSON object, or the empty string.
fn field(node: &Value, name: &str) -> String {
    node.get(name)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// What the destination form starts out filled in with.
///
/// The CSV path follows the chosen rules file: `import/2026/bank.csv.rules`
/// defaults to `import/2026/bank.csv`, which is both the file that rules file
/// describes and the directory the user already keeps statements in. With no
/// candidate — or a rules file not named after a CSV — the upload's own name is
/// used at the journal's root, which is at least somewhere the user can see.
///
/// The journal defaults to the best-ranked writable target, which
/// [`journals::targets`] has already ordered by newest transaction date. No
/// filename is ever inspected; see that module.
fn defaults_for(state: &AppState, upload: &str, rules_id: Option<&str>) -> WireDefaults {
    let snapshot = state.snapshot();
    let targets = journals::targets(&snapshot.journal);
    WireDefaults {
        csv_path: default_csv_path(upload, rules_id),
        journal_id: targets
            .iter()
            .find(|target| target.writable)
            .or_else(|| targets.first())
            .map(|target| target.id.clone()),
    }
}

/// The CSV-destination half of [`defaults_for`], pure so it can be tested as
/// itself rather than against a copy of its own rules restated in a test.
fn default_csv_path(upload: &str, rules_id: Option<&str>) -> String {
    rules_id
        .and_then(|id| id.strip_suffix(".rules"))
        .filter(|stem| ends_with_suffix(stem, ".csv"))
        .map_or_else(
            || {
                let stem = upload.rsplit_once('.').map_or(upload, |(stem, _)| stem);
                let directory = rules_id
                    .and_then(|id| id.rsplit_once('/'))
                    .map_or(String::new(), |(dir, _)| format!("{dir}/"));
                format!("{directory}{stem}.csv")
            },
            str::to_string,
        )
}

/// `POST /api/import/dry-run` — what would be imported, and what it costs.
pub(crate) async fn dry_run(
    State(state): State<AppState>,
    payload: Result<Json<WireDryRunRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    let request = json_body(payload)?;
    let Json(body) = compute(move || run_dry_run(&state, &request)).await?;
    Ok(no_store(body))
}

/// Everything a dry-run and a commit both have to resolve, resolved once.
struct Plan {
    hledger: Hledger,
    staged: std::sync::Arc<Stage>,
    journal: PathBuf,
    rules: PathBuf,
    destination: PathBuf,
    /// The destination's bare file name — what the staged copy is named, and what
    /// `.latest.NAME` is keyed to.
    csv_name: String,
    /// The `--alias` arguments this journal's own `alias` directives become.
    ///
    /// Resolved once, here, so the dry-run and the commit cannot be handed
    /// different sets — see [`import_invocation`].
    aliases: Vec<String>,
    redactor: Redactor,
}

impl Plan {
    /// Resolve every handle in `request`, in the order that puts the cheapest
    /// refusals first.
    fn resolve(state: &AppState, request: &WireDryRunRequest) -> Result<Self, AppError> {
        let staged = resolve_stage(state, &request.stage_id)?;
        let main = main_journal(state, "rules file", &request.rules_id)?;
        let root = include_root(&main)?;
        let (journal, _) = resolve_journal(state, &root, &request.journal_id)?;
        let rules = resolve_rules(&rules::discover(&main), &request.rules_id)?;
        let destination = resolve_destination(&root, &request.csv_path)?;
        let csv_name = file_name(&destination)
            .ok_or_else(|| unresolved("CSV destination", &request.csv_path))?;
        let hledger = resolve_hledger()?;
        let aliases = aliases::arguments(&state.snapshot().journal);

        let redactor = Redactor::default()
            // hledger echoes its own argv[0] in a usage dump — which is what an
            // unrecognised flag produces — and under Nix that is a store path.
            .hide(hledger.path(), "hledger")
            .hide(&journal, &request.journal_id)
            .hide(&rules, &request.rules_id)
            .hide(&destination, &request.csv_path)
            .hide_prefix(&root)
            .hide_prefix(&std::env::temp_dir());
        Ok(Self {
            hledger,
            staged,
            journal,
            rules,
            destination,
            csv_name,
            aliases,
            redactor,
        })
    }

    /// The destination's directory — where `.latest.NAME` lives.
    fn destination_dir(&self) -> &Path {
        self.destination.parent().unwrap_or_else(|| Path::new("."))
    }

    /// The two targets an import touches, each with the handle to report it by.
    fn targets<'a>(&'a self, request: &'a WireDryRunRequest) -> Vec<(&'a Path, &'a str)> {
        vec![
            (self.destination.as_path(), request.csv_path.as_str()),
            (self.journal.as_path(), request.journal_id.as_str()),
        ]
    }
}

/// The whole of `dry-run`, synchronously.
fn run_dry_run(state: &AppState, request: &WireDryRunRequest) -> Result<WireDryRun, AppError> {
    let plan = Plan::resolve(state, request)?;
    let staged = plan
        .staged
        .materialize(
            RUN_WITH_LATEST,
            &plan.csv_name,
            Some(plan.destination_dir()),
        )
        .map_err(stage_failed)?;

    let output = run(
        import_invocation(
            &plan.hledger,
            &plan.journal,
            &plan.rules,
            &staged,
            &plan.aliases,
            true,
        ),
        "run the import preview",
    )?;
    if !output.success() {
        return Ok(WireDryRun::Failed(WireDryRunFailed {
            ok: false,
            stderr: plan.redactor.apply(&output.stderr_lossy()),
        }));
    }

    let entries = output.stdout_lossy();
    let status = output.stderr_lossy();
    let count = count_transactions(&entries)
        .or_else(|| reported_count(&status, "would import"))
        .unwrap_or(0);

    let balance = request
        .balance
        .as_deref()
        .map(|statement| {
            let statement = plain_field(statement, "statement balance")?;
            let account = request
                .balance_account
                .as_deref()
                .map(|account| argument_field(account, "balance account"))
                .transpose()?
                .ok_or_else(|| {
                    AppError::BadRequest(
                        "a statement balance needs the account it is a balance OF".to_string(),
                    )
                })?;
            let computed = verify_balance(&plan.hledger, &plan.journal, &entries, &account)?;
            Ok::<_, AppError>(reconcile(&statement, computed))
        })
        .transpose()?;

    Ok(WireDryRun::Proposed(Box::new(WireProposal {
        ok: true,
        entries: plan.redactor.apply(&entries),
        count,
        status: plan.redactor.apply(&status),
        skipped: skipped_by_dedup(&plan, count)?,
        balance,
        aliases: alias_effect(&plan, &staged, &entries)?,
        blocked_by_git: if autocommit_enabled(&prefs::load()) {
            blocked_by_git(&plan.targets(request))
        } else {
            Vec::new()
        },
    })))
}

/// How many rows `.latest` dedup would silently drop, and from when.
///
/// Measured rather than inferred: the same dry-run is repeated in a run
/// directory with **no** `.latest` beside the CSV, and the difference in
/// transaction counts is exactly what dedup removed. That is rules-agnostic —
/// nothing here has to know which column holds the date, or how the rules file
/// formats it — and it is the only way to be sure, because hledger reports a
/// dropped row nowhere at all.
///
/// `None` when the destination has no dedup state, in which case there is
/// nothing that could have been dropped.
fn skipped_by_dedup(plan: &Plan, count: usize) -> Result<Option<WireSkipped>, AppError> {
    let Some(marker) = stage::latest_marker(plan.destination_dir(), &plan.csv_name) else {
        return Ok(None);
    };
    let bare = plan
        .staged
        .materialize(RUN_BARE, &plan.csv_name, None)
        .map_err(stage_failed)?;
    let output = run(
        import_invocation(
            &plan.hledger,
            &plan.journal,
            &plan.rules,
            &bare,
            &plan.aliases,
            true,
        ),
        "measure what import de-duplication would skip",
    )?;
    if !output.success() {
        return Ok(None);
    }
    let without = count_transactions(&output.stdout_lossy())
        .or_else(|| reported_count(&output.stderr_lossy(), "would import"))
        .unwrap_or(count);
    Ok(without
        .checked_sub(count)
        .filter(|dropped| *dropped > 0)
        .map(|dropped| WireSkipped {
            older_than: marker,
            count: dropped,
        }))
}

/// Which accounts the forwarded aliases rewrote, measured against a second
/// dry-run with none.
///
/// A silent account rewrite immediately before an irreversible write is exactly
/// the thing that has to be on the screen, so this answers with hledger's own
/// before-and-after rather than with our idea of what its regexes do.
///
/// The two runs see the same file, the same rules and the same `.latest`, so
/// their proposals are the same transactions in the same order and the postings
/// line up index for index. A shape mismatch (which would mean the aliases
/// changed how many entries there are — they cannot) answers with no renames
/// rather than with a guess.
///
/// `None` when no alias is in force: there is nothing to say, and no subprocess
/// is spawned to say it.
fn alias_effect(
    plan: &Plan,
    staged: &Path,
    entries: &str,
) -> Result<Option<WireAliasEffect>, AppError> {
    if plan.aliases.is_empty() {
        return Ok(None);
    }
    let output = run(
        import_invocation(&plan.hledger, &plan.journal, &plan.rules, staged, &[], true),
        "measure what the journal's aliases rewrite",
    )?;
    let renames = if output.success() {
        renames_between(&output.stdout_lossy(), entries)
    } else {
        Vec::new()
    };
    Ok(Some(WireAliasEffect {
        forwarded: plan.aliases.len(),
        renames: renames
            .into_iter()
            .map(|(from, to)| WireRename {
                from: plan.redactor.apply(&from),
                to: plan.redactor.apply(&to),
            })
            .collect(),
    }))
}

/// The distinct `(before, after)` account pairs between two proposals of the
/// same import, in first-seen order.
///
/// Our own parser reads both, so this never scrapes hledger's layout — the same
/// decision [`count_transactions`] makes. A text that will not parse yields
/// nothing, because a rename list derived from half a parse is worse than none.
fn renames_between(before: &str, after: &str) -> Vec<(String, String)> {
    /// Enough to see what is happening, few enough to render. A statement with
    /// more distinct accounts than this has a rules file problem, not an alias
    /// one.
    const MAX_RENAMES: usize = 50;
    let read = |text: &str| ledgeline_core::parse_journal(text, "proposed").ok();
    let (Some(before), Some(after)) = (read(before), read(after)) else {
        return Vec::new();
    };
    let mut seen: Vec<(String, String)> = Vec::new();
    for (plain, aliased) in before.transactions.iter().zip(after.transactions.iter()) {
        for (plain, aliased) in plain.postings.iter().zip(aliased.postings.iter()) {
            let pair = (plain.account.0.clone(), aliased.account.0.clone());
            if pair.0 != pair.1 && !seen.contains(&pair) && seen.len() < MAX_RENAMES {
                seen.push(pair);
            }
        }
    }
    seen
}

/// A `500` for a staging-area I/O failure. Only the [`std::io::ErrorKind`] is
/// reported: its `Display` is a fixed phrase from a closed set (`permission
/// denied`, `no space left on device`) and carries no payload at all.
fn stage_failed(error: std::io::Error) -> AppError {
    AppError::Internal(format!(
        "this upload could not be prepared for import: {}",
        error.kind()
    ))
}

/// `POST /api/import/commit` — write the CSV, run the import, report.
pub(crate) async fn commit(
    State(state): State<AppState>,
    payload: Result<Json<WireCommitRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    let request = json_body(payload)?;
    if !state.editing_enabled() {
        return Err(crate::error::editing_disabled());
    }
    // Serialize imports against each other. Two concurrent commits into one
    // journal would interleave hledger's appends and each other's `.latest`
    // writes; held across the `.await` below, which is why it is a tokio mutex.
    // The guard is taken from a CLONE so `state` itself can move into the
    // blocking closure: `AppState::clone` shares every `Arc` inside it, so the
    // mutex the clone locks is the one the closure's state would have locked.
    let guard = state.clone();
    let _write = guard.import_writes().lock().await;
    let Json(body) = compute(move || run_commit(&state, &request)).await?;
    Ok(no_store(body))
}

/// The whole of `commit`, synchronously. Every `?` above the CSV write is a
/// decision not to write anything at all.
fn run_commit(state: &AppState, request: &WireCommitRequest) -> Result<WireCommit, AppError> {
    let plan = Plan::resolve(state, &request.plan)?;
    let targets = plan.targets(&request.plan);
    let prefs = prefs::load();

    // Sequencing rule 3: re-checked HERE. The dry-run's answer is a report, not
    // an authorization, and the UI is not a security boundary.
    let blocked = if autocommit_enabled(&prefs) {
        blocked_by_git(&targets)
    } else {
        Vec::new()
    };
    if !blocked.is_empty() {
        return Err(AppError::Conflict(format!(
            "these files have uncommitted changes, so an import could not be undone with `git \
             revert`: {}. Commit them first, or turn off the git safety net in Preferences.",
            blocked.join(", ")
        )));
    }

    // Sequencing rule 5: a statement balance that would not hold refuses the
    // WHOLE commit, before the CSV is written and before the import runs. See
    // `preflight_assertion`.
    if request.write_assertion {
        preflight_assertion(&plan, &request.plan)?;
    }

    // Sequencing rule 2: the CSV goes to its FINAL destination first, so the
    // import writes `.latest` next to the file it will consult next time.
    let csv = std::fs::read_to_string(plan.staged.data()).map_err(stage_failed)?;
    ledgeline_core::edit::atomic_write(&plan.destination, csv.as_bytes()).map_err(|error| {
        AppError::Internal(format!(
            "{} could not be written: {}. Nothing else was changed.",
            quoted(&request.plan.csv_path),
            error.kind()
        ))
    })?;

    let before = std::fs::read(&plan.journal).map_err(journal_unreadable)?;
    let output = run(
        import_invocation(
            &plan.hledger,
            &plan.journal,
            &plan.rules,
            &plan.destination,
            &plan.aliases,
            false,
        ),
        "run the import",
    )?;
    if !output.success() {
        return Err(AppError::BadRequest(format!(
            "the import failed and nothing was added to {}. hledger said:\n{}",
            quoted(&request.plan.journal_id),
            plan.redactor.apply(&output.stderr_lossy())
        )));
    }
    let after = std::fs::read(&plan.journal).map_err(journal_unreadable)?;
    let imported = appended_count(&before, &after)
        .or_else(|| reported_count(&output.stderr_lossy(), "imported"))
        .unwrap_or(0);

    if request.write_assertion {
        write_assertion(&plan, &request.plan)?;
    }

    let text = std::fs::read_to_string(&plan.journal).map_err(journal_unreadable)?;
    let ordering = match sort::plan(&text) {
        Ok(plan) => WireOrdering {
            in_order: plan.unchanged,
            moves: plan.moves.iter().map(WireMove::from).collect(),
        },
        // A journal this module will not sort (a yearless date) is still a
        // journal we just imported into. Report it as in order rather than
        // failing a commit that has already landed.
        Err(_) => WireOrdering {
            in_order: true,
            moves: Vec::new(),
        },
    };

    let git = autocommit_enabled(&prefs)
        .then(|| {
            commit_targets(
                &targets,
                &commit_message(&request.plan.csv_path, imported),
                &plan.redactor,
            )
        })
        .filter(|result| result.committed || result.message.is_some());

    // The journal changed underneath the editor, so re-open it: otherwise the
    // next transaction edit sees a stale fingerprint and reports a conflict the
    // user did not cause. A failure here is reported to the log rather than to
    // the client — the import itself landed, and the file watcher will retry.
    if let Some(Err(error)) = state.reopen_editor() {
        eprintln!("ledgeline: the journal could not be re-read after an import: {error}");
    }

    Ok(WireCommit {
        csv_written: request.plan.csv_path.clone(),
        journal_written: request.plan.journal_id.clone(),
        imported,
        ordering,
        git,
    })
}

/// The generated commit message: what was imported and how much of it.
fn commit_message(csv_path: &str, imported: usize) -> String {
    let name = csv_path.rsplit('/').next().unwrap_or(csv_path);
    let plural = if imported == 1 { "" } else { "s" };
    format!("import {imported} transaction{plural} from {name}")
}

/// The generated commit message for a CSV kept without an import.
fn save_message(csv_path: &str) -> String {
    let name = csv_path.rsplit('/').next().unwrap_or(csv_path);
    format!("save {name}")
}

/// `POST /api/import/save-csv` — keep the converted CSV, import nothing.
///
/// The other half of "or say plainly that none fit": when no rules file matches
/// a statement, converting it was still worth something and the user gets to
/// keep the result. Nothing here touches a journal, so there is no rules file,
/// no dry-run and no reconciliation — see [`WireSaveCsvRequest`] for why that is
/// a route rather than a mode.
///
/// It takes the same write mutex `commit` does: the two can name the same
/// destination, and the one guarantee a user has about this screen is that
/// exactly the files it names are the files it wrote.
pub(crate) async fn save_csv(
    State(state): State<AppState>,
    payload: Result<Json<WireSaveCsvRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    let request = json_body(payload)?;
    if !state.editing_enabled() {
        return Err(crate::error::editing_disabled());
    }
    let guard = state.clone();
    let _write = guard.import_writes().lock().await;
    let Json(body) = compute(move || run_save_csv(&state, &request)).await?;
    Ok(no_store(body))
}

/// The whole of `save-csv`, synchronously.
///
/// Layers 1-3 are the same as a commit's, minus the two handles this path does
/// not have: the `csvPath` is validated on shape, resolved through
/// [`resolve_destination`] and confined to the include root, and the only bytes
/// written are [`convert::to_csv`]'s own output over a staged upload.
///
/// The git rule is the commit's too, and for the commit's reason: a destination
/// git reports as **modified** blocks, because overwriting somebody's
/// uncommitted edit is the one thing `git diff` could not have undone. An
/// untracked CSV — the ordinary case, a statement saved for the first time —
/// does not block.
fn run_save_csv(state: &AppState, request: &WireSaveCsvRequest) -> Result<WireSavedCsv, AppError> {
    let staged = resolve_stage(state, &request.stage_id)?;
    let main = main_journal(state, "CSV destination", &request.csv_path)?;
    let root = include_root(&main)?;
    let destination = resolve_destination(&root, &request.csv_path)?;
    let redactor = Redactor::default()
        .hide(&destination, &request.csv_path)
        .hide_prefix(&root)
        .hide_prefix(&std::env::temp_dir());

    let prefs = prefs::load();
    let targets = vec![(destination.as_path(), request.csv_path.as_str())];
    if autocommit_enabled(&prefs) {
        let blocked = blocked_by_git(&targets);
        if !blocked.is_empty() {
            return Err(AppError::Conflict(format!(
                "these files have uncommitted changes, so overwriting them could not be undone \
                 with `git revert`: {}. Commit them first, or turn off the git safety net in \
                 Preferences.",
                blocked.join(", ")
            )));
        }
    }

    let csv = std::fs::read_to_string(staged.data()).map_err(stage_failed)?;
    ledgeline_core::edit::atomic_write(&destination, csv.as_bytes()).map_err(|error| {
        AppError::Internal(format!(
            "{} could not be written: {}. Nothing else was changed.",
            quoted(&request.csv_path),
            error.kind()
        ))
    })?;

    let git = autocommit_enabled(&prefs)
        .then(|| commit_targets(&targets, &save_message(&request.csv_path), &redactor))
        .filter(|result| result.committed || result.message.is_some());
    Ok(WireSavedCsv {
        csv_written: request.csv_path.clone(),
        git,
    })
}

/// How many transactions `hledger import` appended, counted from the bytes it
/// added.
///
/// Exact, and derived from our own parser rather than from hledger's prose. The
/// prefix check is what makes it safe: if the file did not simply grow — which
/// would mean something other than this import rewrote it — this declines and the
/// caller falls back to the status line.
fn appended_count(before: &[u8], after: &[u8]) -> Option<usize> {
    let appended = after
        .len()
        .checked_sub(before.len())
        .filter(|_| after.starts_with(before))
        .map(|_| &after[before.len()..])?;
    count_transactions(&String::from_utf8_lossy(appended))
}

/// A `500` for a journal we could not read back. Only the kind, never the path.
fn journal_unreadable(error: std::io::Error) -> AppError {
    AppError::Internal(format!(
        "the journal could not be read back after the import: {}",
        error.kind()
    ))
}

/// The statement balance and the account it is a balance of, both validated, or
/// `None` when this request carried no balance at all.
fn assertion_fields(request: &WireDryRunRequest) -> Result<Option<(String, String)>, AppError> {
    let (Some(statement), Some(account)) = (&request.balance, &request.balance_account) else {
        return Ok(None);
    };
    Ok(Some((
        plain_field(statement, "statement balance")?,
        argument_field(account, "balance account")?,
    )))
}

/// The newest transaction date in `text`, as our own parser reads it.
fn newest_date(text: &str) -> Option<String> {
    ledgeline_core::parse_journal(text, "journal")
        .ok()?
        .transactions
        .into_iter()
        .map(|txn| txn.date)
        .max()
}

/// One assertion transaction, in the shape `hledger close --assert` produces —
/// **carrying its commodity**.
///
/// # Why the commodity is the whole of this function
///
/// `assets:bank:checking  0 = 2949.80` is not an assertion that the account
/// holds 2949.80 dollars. An amount with no commodity is an amount in the
/// *empty* commodity, so hledger reads that line as "the balance in commodity
/// `""` is 2949.80", computes 0, and the assertion fails — on the import that
/// wrote it and on every `hledger check` afterwards. Verified against hledger
/// 1.52, with the journal in `$` and `2949.80` typed into the form:
///
/// ```text
/// 4 |     assets:bank:checking               0 = 2949.80
/// Balance assertion failed in assets:bank:checking
/// In commodity ""  ... the asserted balance is: 2949.80
///                      but the calculated balance is: 0
/// ```
///
/// That is **fact 4** of `plans/11-enhanced-import.md` — the trap
/// `rules/matching.rs` exists to score other people's rules files against —
/// arriving in output this module writes itself. A statement balance is typed by
/// hand into a form whose placeholder is `2945.05`, so a bare number is the
/// normal case rather than the exotic one.
///
/// So the commodity is resolved in this order:
///
/// 1. **what the user typed.** `$2949.80` is them being explicit and is used as
///    they wrote it, even if it disagrees with the journal — in which case
///    `hledger check` refuses it below and says so in its own words, which is
///    better than us quietly overriding what they asked for.
/// 2. **the commodity hledger computed for that account**, over the journal plus
///    the entries not yet in it. This is the journal's own commodity for the very
///    account being asserted, which is the only commodity the assertion could
///    hold in. A *bare* computed balance is an answer too, not a failure: a
///    journal that genuinely carries no commodity wants a bare assertion and
///    gets one.
/// 3. otherwise the assertion is **REFUSED**. A silently-wrong assertion is
///    worse than no assertion: it is one line, in a file the user did not write,
///    that fails every check from then on.
fn assertion_lines(
    date: &str,
    account: &str,
    statement: &Amount,
    computed: Option<&Amount>,
) -> Result<String, AppError> {
    let commodity = if statement.has_commodity() {
        statement
    } else {
        computed.ok_or_else(|| {
            AppError::BadRequest(
                "Ledgeline could not tell which currency this balance is in, so it did not write \
                 an assertion: hledger has no balance for that account to take one from. An \
                 amount with no currency asserts a balance in a currency of its own, which would \
                 fail every later `hledger check`. Type the balance with its symbol (for example \
                 `$2945.05`), or check the account name."
                    .to_string(),
            )
        })?
    };
    let sign = if statement.negative { "-" } else { "" };
    let asserted = commodity.wrap(&format!("{sign}{}", statement.number));
    // `0` as the posting amount is what `hledger close --assert` writes: the
    // transaction balances trivially. It carries the commodity too, so the
    // posting and its assertion are amounts in the same one.
    Ok(format!(
        "{date} assert balances  ; assert:\n    {account}    {} = {asserted}\n",
        commodity.wrap("0")
    ))
}

/// The assertion transaction this request asks for, decided against the journal
/// **as it will stand** once `proposed` is part of it.
///
/// `proposed` is the entries that are not in the file yet: the dry-run's own
/// stdout when this runs before the import, and empty afterwards. Both the date
/// and the commodity are read from that combined view, so the pre-flight check
/// and the write produce the same transaction rather than two that could differ.
///
/// The date is the newest transaction date rather than today's: the assertion is
/// a statement about the balance *after everything in this file*, and taking it
/// from the data makes the write deterministic and the test for it stable.
fn plan_assertion(
    plan: &Plan,
    request: &WireDryRunRequest,
    proposed: &str,
) -> Result<Option<String>, AppError> {
    let Some((statement, account)) = assertion_fields(request)? else {
        return Ok(None);
    };
    let text = std::fs::read_to_string(&plan.journal).map_err(journal_unreadable)?;
    let date = newest_date(&format!("{text}\n{proposed}")).ok_or_else(|| {
        AppError::BadRequest(
            "a balance assertion needs at least one transaction to be dated after".to_string(),
        )
    })?;
    let typed = split_amount(&statement).ok_or_else(|| {
        AppError::BadRequest(format!(
            "{} is not a balance Ledgeline can assert: it must be one amount, written with `.` for \
             the decimal point",
            quoted(&statement)
        ))
    })?;
    let computed = verify_balance(&plan.hledger, &plan.journal, proposed, &account)?
        .as_deref()
        .and_then(split_amount);
    assertion_lines(&date, &account, &typed, computed.as_ref()).map(Some)
}

/// Put `assertion` to `hledger check` as ONE journal — the file, the entries not
/// yet in it, and the assertion — by the same one-`-f` mechanism
/// [`verify_balance`] uses, never two `-f` flags (fact 3).
///
/// `Ok(None)` means it holds. `Ok(Some(stderr))` means hledger refused it and
/// carries its own words, redacted; the caller phrases what that cost.
fn check_assertion(
    plan: &Plan,
    proposed: &str,
    assertion: &str,
) -> Result<Option<String>, AppError> {
    let output = run(
        plan.hledger
            .invoke(["-f", "-", "check"])
            .stdin(concatenated(&plan.journal, &format!("{proposed}\n{assertion}")).into_bytes())
            .timeout(BALANCE_TIMEOUT),
        "check the statement balance",
    )?;
    Ok((!output.success()).then(|| plan.redactor.apply(&output.stderr_lossy())))
}

/// Refuse a statement balance that would not hold — **before anything at all is
/// written**.
///
/// This runs against the dry-run's own proposed entries, which are exactly what
/// the real import is about to append, so it can answer the question the commit
/// used to answer only afterwards: the import was applied and *then* the
/// assertion failed, leaving the client an error and the journal changed. That
/// is honest but not recoverable in one click. Here the whole commit is
/// all-or-nothing for the failure users actually hit — a mistyped balance —
/// while [`write_assertion`] still re-checks the exact bytes it appends, because
/// what makes it safe to write is that hledger agreed to *that text*.
fn preflight_assertion(plan: &Plan, request: &WireDryRunRequest) -> Result<(), AppError> {
    if assertion_fields(request)?.is_none() {
        return Ok(());
    }
    let staged = plan
        .staged
        .materialize(
            RUN_WITH_LATEST,
            &plan.csv_name,
            Some(plan.destination_dir()),
        )
        .map_err(stage_failed)?;
    let output = run(
        import_invocation(
            &plan.hledger,
            &plan.journal,
            &plan.rules,
            &staged,
            &plan.aliases,
            true,
        ),
        "preview the import",
    )?;
    // A dry-run hledger refuses means the real import is about to refuse for the
    // same reason, and its message is the one that names the offending row.
    // Nothing has been written either way, so let it speak.
    if !output.success() {
        return Ok(());
    }
    let entries = output.stdout_lossy();
    let Some(assertion) = plan_assertion(plan, request, &entries)? else {
        return Ok(());
    };
    match check_assertion(plan, &entries, &assertion)? {
        None => Ok(()),
        Some(stderr) => Err(AppError::BadRequest(format!(
            "the statement balance does not match what this import would produce, so NOTHING was \
             written — not the CSV, not the journal. hledger said:\n{stderr}"
        ))),
    }
}

/// Append the statement balance as an assertion transaction.
///
/// **Verified before it is written**, not after: the exact transaction is handed
/// to `hledger check` on stdin, concatenated with the journal. So a balance that
/// does not reconcile leaves the journal exactly as the import produced it,
/// rather than gaining a line that makes every subsequent `hledger check` fail.
/// [`preflight_assertion`] has normally refused already, before the import ran at
/// all; this is what guarantees the property for the bytes that actually land.
fn write_assertion(plan: &Plan, request: &WireDryRunRequest) -> Result<(), AppError> {
    let Some(assertion) = plan_assertion(plan, request, "")? else {
        return Ok(());
    };
    if let Some(stderr) = check_assertion(plan, "", &assertion)? {
        return Err(AppError::BadRequest(format!(
            "the statement balance does not match the journal, so no assertion was written. The \
             import itself was applied. hledger said:\n{stderr}"
        )));
    }

    let text = std::fs::read_to_string(&plan.journal).map_err(journal_unreadable)?;
    let separator = if text.ends_with('\n') { "\n" } else { "\n\n" };
    ledgeline_core::edit::atomic_write(
        &plan.journal,
        format!("{text}{separator}{assertion}").as_bytes(),
    )
    .map_err(|error| {
        AppError::Internal(format!(
            "the balance assertion could not be written: {}. The import itself was applied.",
            error.kind()
        ))
    })
}

/// `POST /api/import/sort` — the confirmed format-preserving re-sort.
///
/// Offered only after a commit reported `inOrder: false`, and re-planned here:
/// [`sort::apply`] recomputes the sort from the text it is handed and refuses if
/// it does not match the plan, so the bytes written are always the sort of the
/// file as it stands.
pub(crate) async fn sort_journal(
    State(state): State<AppState>,
    payload: Result<Json<WireSortRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    let request = json_body(payload)?;
    if !state.editing_enabled() {
        return Err(crate::error::editing_disabled());
    }
    let guard = state.clone();
    let _write = guard.import_writes().lock().await;
    let Json(body) = compute(move || run_sort(&state, &request)).await?;
    Ok(no_store(body))
}

/// The whole of `sort`, synchronously.
fn run_sort(state: &AppState, request: &WireSortRequest) -> Result<WireSorted, AppError> {
    let main = main_journal(state, "journal", &request.journal_id)?;
    let root = include_root(&main)?;
    let (journal, _) = resolve_journal(state, &root, &request.journal_id)?;

    let text = std::fs::read_to_string(&journal).map_err(journal_unreadable)?;
    let plan = sort::plan(&text).map_err(|error| AppError::BadRequest(error.to_string()))?;
    if plan.unchanged {
        return Ok(WireSorted { moved: 0 });
    }
    let sorted =
        sort::apply(&text, &plan).map_err(|error| AppError::BadRequest(error.to_string()))?;
    ledgeline_core::edit::atomic_write(&journal, sorted.as_bytes()).map_err(|error| {
        AppError::Internal(format!(
            "{} could not be re-sorted: {}. Nothing was changed.",
            quoted(&request.journal_id),
            error.kind()
        ))
    })?;
    if let Some(Err(error)) = state.reopen_editor() {
        eprintln!("ledgeline: the journal could not be re-read after a sort: {error}");
    }
    Ok(WireSorted {
        moved: plan.moves.len(),
    })
}

/// `GET /api/prefs` — the stored preferences.
pub(crate) async fn prefs_get() -> Result<Response, AppError> {
    let Json(body) = compute(|| Ok(WirePrefs::from(&prefs::load()))).await?;
    Ok(no_store(body))
}

/// `PUT /api/prefs` — replace them.
///
/// `hledgerPath` is validated by the store and a bad value is rejected with a
/// `400` rather than persisted: a path that fails at *import* time, several
/// screens later, as "could not run hledger" is the mysterious-failure shape the
/// preferences module exists to prevent.
pub(crate) async fn prefs_put(
    payload: Result<Json<WirePrefs>, JsonRejection>,
) -> Result<Response, AppError> {
    let request = json_body(payload)?;
    let Json(body) = compute(move || {
        let prefs = Prefs {
            hledger_path: request.hledger_path.as_deref().map(PathBuf::from),
            git_autocommit: request.git_autocommit,
        };
        prefs::store(&prefs).map_err(|error| AppError::BadRequest(error.to_string()))?;
        Ok(WirePrefs::from(&prefs))
    })
    .await?;
    Ok(no_store(body))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The syntactic gate in full. Every rejected shape is one that must never
    /// reach the filesystem; every accepted one is something a scan or a
    /// projection could genuinely have produced.
    #[test]
    fn a_relative_handle_is_accepted_only_in_the_shape_a_scan_produces() {
        for id in [
            "bank.csv",
            "import/2026/bank.csv",
            "a/b/c/d/e/f/g/h/deep.csv",
            "Bank.CSV",
            "kaffee-über.csv",
        ] {
            assert!(
                validate_relative_id(id, "CSV destination", Some(".csv")).is_ok(),
                "{id} should be accepted"
            );
        }

        for id in [
            "",
            "../escape.csv",
            "a/../b.csv",
            "./a.csv",
            "/etc/passwd.csv",
            "a\\b.csv",
            "C:/x.csv",
            "a.csv:stream",
            "x.txt",
            ".csv",
            "a//b.csv",
            "a/b/c/d/e/f/g/h/i/j.csv",
            "a\u{0}.csv",
            "a\n.csv",
        ] {
            assert!(
                validate_relative_id(id, "CSV destination", Some(".csv")).is_err(),
                "{id:?} should be rejected"
            );
        }

        // A journal handle has no required suffix — real journals are `.journal`,
        // `.hledger`, `.ledger` and `.j` — but every other rule still applies.
        assert!(validate_relative_id("2026/2026.hledger", "journal id", None).is_ok());
        assert!(validate_relative_id("../secrets.journal", "journal id", None).is_err());
    }

    /// The upload header is REFUSED rather than stripped. Silently turning
    /// `../../.bashrc` into `.bashrc` hides both the attack and the bug.
    #[test]
    fn a_path_shaped_filename_header_is_refused() {
        let header = |value: &str| {
            let mut headers = HeaderMap::new();
            headers.insert(
                HeaderName::from_static(FILENAME_HEADER),
                HeaderValue::from_str(value).expect("test header"),
            );
            headers
        };
        assert_eq!(
            bare_filename(&header("bank.csv")).as_deref(),
            Ok("bank.csv")
        );
        assert_eq!(
            bare_filename(&header("  bank.csv  ")).as_deref(),
            Ok("bank.csv")
        );
        for bad in [
            "../../.bashrc",
            "/etc/passwd",
            "sub/bank.csv",
            "a\\b.csv",
            "C:bank.csv",
            "..",
            ".",
            "",
            "   ",
        ] {
            assert!(
                bare_filename(&header(bad)).is_err(),
                "{bad:?} must be refused"
            );
        }
        // No header at all is its own refusal, not a default.
        assert!(bare_filename(&HeaderMap::new()).is_err());
    }

    /// Redaction is longest-first, so a file inside the root is rewritten to its
    /// own handle rather than being half-rewritten by the root's entry.
    #[test]
    fn redaction_prefers_the_most_specific_path() {
        let redactor = Redactor::default()
            .hide(Path::new("/books/2026/2026.journal"), "2026/2026.journal")
            .hide_prefix(Path::new("/books"));
        assert_eq!(
            redactor.apply("imported 3 transactions to /books/2026/2026.journal"),
            "imported 3 transactions to 2026/2026.journal"
        );
        assert_eq!(
            redactor.apply("could not read /books/other.csv"),
            "could not read other.csv"
        );
        assert_eq!(redactor.apply("in /books"), "in .");
    }

    /// The count comes from OUR parse of hledger's stdout, and only falls back
    /// to its prose when that fails.
    #[test]
    fn the_transaction_count_is_parsed_not_scraped() {
        let entries = "2026-02-01 GROCERY\n    expenses:food  $1.00\n    assets:cash  $-1.00\n\n\
                       2026-02-02 FUEL\n    expenses:car  $2.00\n    assets:cash  $-2.00\n";
        assert_eq!(count_transactions(entries), Some(2));
        assert_eq!(count_transactions("this is not a journal at all @@@"), None);

        assert_eq!(
            reported_count(
                "would import 3 new transactions from bank.csv:\n",
                "would import"
            ),
            Some(3)
        );
        assert_eq!(
            reported_count(
                "imported 12 new transactions from bank.csv to x\n",
                "imported"
            ),
            Some(12)
        );
        assert_eq!(reported_count("nothing to say\n", "would import"), None);
    }

    /// The balance reader takes machine output only, and says "I do not know"
    /// rather than guessing at a shape it was not given.
    #[test]
    fn the_balance_is_read_from_csv_output() {
        let csv = "\"account\",\"balance\"\n\"assets:bank:checking\",\"$2945.05\"\n";
        assert_eq!(balance_from_csv(csv).as_deref(), Some("$2945.05"));
        // A query that matched nothing is a real answer, not a zero.
        assert_eq!(balance_from_csv("\"account\",\"balance\"\n"), None);
        assert_eq!(balance_from_csv(""), None);
    }

    /// Exact-decimal reconciliation, including the shapes a bank actually
    /// produces and the ones this parser must decline.
    #[test]
    fn money_round_trips_through_the_exact_decimal() {
        let parse_money = |text: &str| split_amount(text).and_then(|amount| amount.value());
        let render = |value: Dec| render_money_at(value, value.places);
        for (text, rendered) in [
            ("$2945.05", "2945.05"),
            ("2945.05", "2945.05"),
            ("$2,945.05", "2945.05"),
            ("-3238.65", "-3238.65"),
            ("2945.05 USD", "2945.05"),
            ("0", "0"),
        ] {
            let parsed = parse_money(text).unwrap_or_else(|| panic!("{text} should parse"));
            assert_eq!(render(parsed), rendered, "{text}");
        }
        assert_eq!(render(parse_money("$-5.00").expect("signed")), "-5.00");

        // A MULTI-COMMODITY balance is not a number we may subtract, and saying
        // so beats reporting a plausible wrong one. This is the case that has
        // already been got wrong once: stripping `,` as a thousands separator
        // first turns `$10.00, 3 AAPL` into `10.003`.
        for ambiguous in [
            "$10.00, 3 AAPL",
            "3 AAPL, $10.00",
            "$10.00 3 AAPL",
            "1.234.567,89",
            "",
            "$",
            "USD",
        ] {
            assert!(
                parse_money(ambiguous).is_none(),
                "{ambiguous:?} must not be read as one amount"
            );
        }
    }

    /// The reconciliation's three fields have to agree with each other: a zero
    /// difference is exactly a match.
    #[test]
    fn a_reconciliation_matches_only_when_the_difference_is_zero() {
        let matched = reconcile("2945.05", Some("$2945.05".to_string()));
        assert!(matched.matches);
        assert_eq!(matched.difference.as_deref(), Some("$0.00"));

        let off = reconcile("2945.05", Some("$1950.05".to_string()));
        assert!(!off.matches);
        assert_eq!(off.difference.as_deref(), Some("$995.00"));

        // An unreadable computed balance reports no difference rather than a
        // wrong one, and never claims a match.
        let unknown = reconcile("2945.05", Some("$10.00, 3 AAPL".to_string()));
        assert!(!unknown.matches);
        assert_eq!(unknown.difference, None);
        assert_eq!(unknown.statement, "2945.05", "both sides stay verbatim");
        assert_eq!(unknown.computed, "$10.00, 3 AAPL");

        // A balance hledger could not compute AT ALL is an absence, not a zero:
        // `computed: "0"` here would claim the account is empty and hand the
        // user the whole statement amount as the difference.
        let uncomputed = reconcile("2945.05", None);
        assert!(!uncomputed.matches);
        assert_eq!(uncomputed.computed, "");
        assert_eq!(uncomputed.difference, None);
    }

    /// **Both sides of a reconciliation are comparable.** The user types a bare
    /// number into a form whose placeholder is `2945.05`; hledger answers
    /// `$2945.05`. Rendering those two beside each other unchanged is a screen
    /// that says "match" above two strings that visibly differ.
    #[test]
    fn a_reconciliation_puts_both_sides_in_one_representation() {
        let matched = reconcile("2945.05", Some("$2945.05".to_string()));
        assert_eq!(matched.statement, "$2945.05");
        assert_eq!(matched.computed, "$2945.05");
        assert!(matched.matches);

        // Scale is shared too: `2945.8` typed against `$2945.80` computed is one
        // number, and must not read as two.
        let scaled = reconcile("2945.8", Some("$2945.80".to_string()));
        assert_eq!(scaled.statement, "$2945.80");
        assert_eq!(scaled.computed, "$2945.80");
        assert!(scaled.matches);

        // A trailing commodity is carried on the same side hledger put it.
        let suffixed = reconcile("2945.05", Some("2945.05 USD".to_string()));
        assert_eq!(suffixed.statement, "2945.05 USD");
        assert_eq!(suffixed.difference.as_deref(), Some("0.00 USD"));

        // Digit grouping is normalised away, so `$2,945.05` and `2945.05` do not
        // read as different numbers either.
        let grouped = reconcile("2,945.05", Some("$2945.05".to_string()));
        assert_eq!(grouped.statement, "$2945.05");
        assert!(grouped.matches);

        // A journal that genuinely has no commodity keeps a bare pair; nothing
        // is invented to make them look richer than they are.
        let bare = reconcile("2945.05", Some("2945.05".to_string()));
        assert_eq!(bare.statement, "2945.05");
        assert_eq!(bare.computed, "2945.05");

        // A negative statement keeps its sign inside the commodity, the way
        // hledger writes one.
        let negative = reconcile("-3238.65", Some("$-3238.65".to_string()));
        assert_eq!(negative.statement, "$-3238.65");
        assert!(negative.matches);
    }

    /// A money string splits into a commodity and a numeral, and the commodity
    /// survives — which is the whole of the assertion fix.
    #[test]
    fn an_amount_keeps_the_commodity_around_it() {
        let dollars = split_amount("$2,945.05").expect("a prefixed amount");
        assert!(dollars.has_commodity());
        assert_eq!(dollars.wrap("0"), "$0");
        assert_eq!(dollars.prefix, "$");

        let suffixed = split_amount("2945.05 USD").expect("a suffixed amount");
        assert!(suffixed.has_commodity());
        assert_eq!(suffixed.wrap("0"), "0 USD");

        // The sign is not part of the commodity, on either spelling.
        for signed in ["$-5.00", "-$5.00"] {
            let amount = split_amount(signed).expect(signed);
            assert_eq!(amount.prefix, "$", "{signed}");
            assert!(amount.negative, "{signed}");
            assert_eq!(amount.wrap("0"), "$0", "{signed}");
        }

        // A BARE amount is the trap, and it has to be visible as one rather than
        // read as "dollars, presumably".
        let bare = split_amount("2945.05").expect("a bare amount");
        assert!(!bare.has_commodity());
        assert_eq!(bare.wrap("0"), "0");
    }

    /// **The regression for fact 4 in our own output.** A statement balance with
    /// no currency symbol must not be written as a bare assertion: that asserts a
    /// balance in the empty commodity, which hledger computes as 0, and the
    /// assertion fails on every check from then on.
    #[test]
    fn an_assertion_carries_the_commodity_of_the_computed_balance() {
        let bare = split_amount("2949.80").expect("the user's number");
        let computed = split_amount("$2949.80").expect("hledger's answer");

        let written = assertion_lines("2026-01-20", "assets:bank:checking", &bare, Some(&computed))
            .expect("a commodity was available");
        assert!(
            written.contains("assets:bank:checking    $0 = $2949.80"),
            "the assertion must carry the commodity on BOTH amounts: {written}"
        );
        assert!(
            written.starts_with("2026-01-20 assert balances  ; assert:\n"),
            "the `hledger close --assert` shape: {written}"
        );

        // What the user typed WINS: they were explicit, and hledger's own check
        // is what tells them if they were wrong.
        let typed = split_amount("€2949.80").expect("an explicit commodity");
        let explicit = assertion_lines("2026-01-20", "assets:bank", &typed, Some(&computed))
            .expect("the user's own commodity");
        assert!(explicit.contains("€0 = €2949.80"), "{explicit}");

        // A journal that genuinely carries no commodity gets a bare assertion,
        // which is correct there and is not the same as not knowing.
        let bare_journal = split_amount("2949.80").expect("a bare computed balance");
        let plain = assertion_lines("2026-01-20", "assets:bank", &bare, Some(&bare_journal))
            .expect("a bare journal is an answer");
        assert!(plain.contains("assets:bank    0 = 2949.80"), "{plain}");

        // And with NOTHING to take a commodity from, the assertion is REFUSED.
        // A silently-wrong assertion is worse than none: it is a line the user
        // did not write that fails every later `hledger check`.
        let refused = assertion_lines("2026-01-20", "assets:bank", &bare, None);
        assert!(refused.is_err(), "a bare assertion must never be written");
        assert!(
            refused.unwrap_err().to_string().contains("currency"),
            "the refusal must say what is missing"
        );
    }

    /// The destination default follows the rules file the user picked, which is
    /// both the file that rules file describes and a directory they already keep
    /// statements in.
    #[test]
    fn the_csv_default_follows_the_chosen_rules_file() {
        assert_eq!(
            default_csv_path("statement.ofx", Some("import/2026/bank.csv.rules")),
            "import/2026/bank.csv"
        );
        // A rules file not named after a CSV falls back to the upload's own name
        // in that rules file's directory.
        assert_eq!(
            default_csv_path("statement.ofx", Some("import/2026/bank.rules")),
            "import/2026/statement.csv"
        );
        assert_eq!(default_csv_path("statement.ofx", None), "statement.csv");

        // Whatever it derives must be something `resolve_destination` will then
        // accept, or the form starts out un-submittable.
        for (upload, rules) in [
            ("statement.ofx", Some("import/2026/bank.csv.rules")),
            ("statement.ofx", None),
            ("bank.csv", None),
        ] {
            let derived = default_csv_path(upload, rules);
            assert!(
                validate_relative_id(&derived, "CSV destination", Some(".csv")).is_ok(),
                "the default {derived:?} must itself be a valid destination"
            );
        }
    }

    /// The commit message names the file and the count, and never a path.
    #[test]
    fn the_commit_message_names_the_statement_and_the_count() {
        assert_eq!(
            commit_message("import/2026/bank.csv", 3),
            "import 3 transactions from bank.csv"
        );
        assert_eq!(
            commit_message("bank.csv", 1),
            "import 1 transaction from bank.csv"
        );
    }

    /// The concatenation is ONE journal with ONE `-f`, and the journal is
    /// reached by an absolute `include` so its own relative includes still
    /// resolve. See [`verify_balance`] for why.
    #[test]
    fn the_concatenation_is_one_journal_reached_absolutely() {
        let text = concatenated(Path::new("/books/main.journal"), "2026-02-01 X\n");
        assert!(
            text.starts_with("include /books/main.journal\n"),
            "the journal must be included by absolute path: {text:?}"
        );
        assert!(text.contains("2026-02-01 X"));
    }

    /// The appended-bytes count is exact, and declines when the file did not
    /// simply grow — which would mean something other than this import rewrote
    /// it.
    #[test]
    fn the_imported_count_is_the_bytes_the_import_added() {
        let before = b"2026-01-01 A\n    a  $1.00\n    b  $-1.00\n";
        let after = b"2026-01-01 A\n    a  $1.00\n    b  $-1.00\n\n2026-02-01 B\n    a  $2.00\n    b  $-2.00\n";
        assert_eq!(appended_count(before, after), Some(1));
        assert_eq!(appended_count(before, before), Some(0));
        assert_eq!(appended_count(before, b"something else entirely\n"), None);
    }

    /// A hostile handle must not be able to put a control character, or a
    /// megabyte of text, into a user-facing error body.
    #[test]
    fn quoted_escapes_control_characters_and_clips() {
        assert_eq!(quoted("a\u{0}b"), "\"a\\0b\"");
        let huge = "x".repeat(10_000);
        let rendered = quoted(&huge);
        assert!(rendered.len() < 200, "an error body must stay small");
        assert!(
            rendered.ends_with('…'),
            "a clip must be visible: {rendered}"
        );
    }

    /// The hledger reason codes are a closed set the UI switches on.
    #[test]
    fn every_hledger_failure_has_its_own_reason_code() {
        let cases = [
            (HledgerError::NotFound, "notFound"),
            (
                HledgerError::TooOld {
                    found: crate::hledger::Version {
                        major: 1,
                        minor: 30,
                    },
                    min: crate::hledger::MIN_HLEDGER,
                },
                "tooOld",
            ),
            (HledgerError::Unrunnable, "unrunnable"),
            (
                HledgerError::TimedOut {
                    after: Duration::from_secs(30),
                },
                "timedOut",
            ),
        ];
        for (error, expected) in cases {
            assert_eq!(hledger_reason(&error), expected);
            // Every one of them is a 501 with an actionable sentence, never a 500.
            let rendered = hledger_unavailable(&error);
            assert_eq!(
                rendered.status(),
                axum::http::StatusCode::NOT_IMPLEMENTED,
                "{error}"
            );
            assert!(rendered.to_string().contains("Preferences"), "{error}");
        }
    }

    /// The two field rules, and why they are two.
    ///
    /// A statement balance is written into a journal; a balance account becomes
    /// a positional argument to `hledger balance`. Only the second may not be
    /// option-shaped — and only the FIRST may be negative, which is the whole
    /// reason they are not one function.
    #[test]
    fn a_plain_field_refuses_what_could_not_be_one() {
        assert_eq!(
            plain_field(" 2945.05 ", "balance").as_deref(),
            Ok("2945.05")
        );
        for bad in [
            "",
            "   ",
            "a\nb",
            "a\u{0}b",
            &"x".repeat(MAX_FIELD_CHARS + 1),
        ] {
            assert!(
                plain_field(bad, "balance").is_err(),
                "{bad:?} must be refused"
            );
        }

        // A CREDIT-CARD statement balance is NEGATIVE, and `-3238.65` is the
        // WP-11 contract's own example. Refusing a leading `-` here would make
        // the reconciliation unusable for exactly the accounts people most want
        // to reconcile.
        assert_eq!(
            plain_field("-3238.65", "balance").as_deref(),
            Ok("-3238.65")
        );
        assert_eq!(
            reconcile("-3238.65", Some("$-3238.65".to_string()))
                .difference
                .as_deref(),
            // In the commodity hledger reported, like every other field of the
            // reconciliation — see `reconcile`.
            Some("$0.00"),
            "a negative statement balance must reconcile"
        );

        // An hledger ARGUMENT is the option-shaped-sensitive one: a query takes
        // no `--` terminator, so hledger's own parser would read `-f` as a flag.
        assert_eq!(
            argument_field("assets:bank:checking", "account").as_deref(),
            Ok("assets:bank:checking")
        );
        assert!(argument_field("-f", "account").is_err());
        assert!(argument_field("--depth", "account").is_err());
    }
}
