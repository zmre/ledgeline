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
//!    destination's `.latest.NAME` copied in beside it — verbatim, and if it
//!    cannot be, the import is refused.** `hledger import` de-duplicates from a
//!    state file kept next to the *data file* and keyed to its name. A dry-run
//!    against a temp copy called anything else consults a state file that does
//!    not exist, reports every row as new, and then the real import silently
//!    drops the back-dated ones. The file is **not one date** — it repeats the
//!    newest date once per record sharing it, and the repeat count is what
//!    hledger skips by — so it is never parsed or rewritten on the way. A state
//!    file that is there and unreadable stops the import
//!    ([`refuse_unusable_dedup_state`]) rather than quietly becoming "there is
//!    none". See [`stage`](crate::stage).
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
use ledgeline_core::decimal::{Dec, MAX_RENDER_PLACES};
use ledgeline_core::edit::Fingerprint;
use ledgeline_core::hledger_conf;
use ledgeline_core::journals::{self, JournalTarget};
use ledgeline_core::model::Status;
use ledgeline_core::reimport::{self, RowClassification};
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

/// How many status changes / conflicts one response lists individually.
///
/// The lists exist to be read, and nobody reads two thousand of them; a user who
/// reformatted their journal could otherwise turn every row of a year's
/// statement into a conflict entry. The counts beside each list are **not**
/// capped, so the number is always the true one and only the detail is bounded —
/// the same trade `PREVIEW_ROWS` makes. It bounds the reporting only: the status
/// flips a commit actually applies are not capped, because a cap on those would
/// silently leave a statement half-synced.
const MAX_ID_REPORTS: usize = 200;

/// How many field disagreements one conflicting row lists. A transaction has a
/// handful of fields and a couple of postings; past that the row is not "a
/// changed amount" but "a different transaction", and the first few say so.
const MAX_ID_DIFFS: usize = 8;

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

/// hledger's `-I`, spelled out.
///
/// It goes on **exactly one** invocation — [`import_invocation`], the only one
/// that reads a target file in isolation — and the argument for it is at that
/// function, at length, because the next reader's instinct will be to delete it.
///
/// [`verify_balance`] and [`check_assertion`] deliberately carry neither, and
/// `no_balance_invocation_ignores_assertions` pins that.
///
/// The long spelling is used so the argv says what it does; `-I` is the same
/// flag and hledger accepts both.
const IGNORE_ASSERTIONS: &str = "--ignore-assertions";

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
    StatementChosen { of: usize },
    DatesFromSerial { count: usize },
    EncodingGuessed { label: String },
    DelimiterSniffed { delimiter: String },
    PreambleSkipped { lines: usize },
    TrailerSkipped { lines: usize },
    BlankRowsDropped { count: usize },
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
            ConvertNote::StatementChosen { of } => Self::StatementChosen { of: *of },
            ConvertNote::DatesFromSerial { count } => Self::DatesFromSerial { count: *count },
            ConvertNote::EncodingGuessed { label } => Self::EncodingGuessed {
                label: label.clone(),
            },
            ConvertNote::DelimiterSniffed { delimiter } => Self::DelimiterSniffed {
                delimiter: delimiter.to_string(),
            },
            ConvertNote::PreambleSkipped { lines } => Self::PreambleSkipped { lines: *lines },
            ConvertNote::TrailerSkipped { lines } => Self::TrailerSkipped { lines: *lines },
            ConvertNote::BlankRowsDropped { count } => Self::BlankRowsDropped { count: *count },
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
    /// The `ledgeline import …` command line that reproduces this import
    /// non-interactively, for the panel's copy affordance.
    ///
    /// **Contract amendment (WP-16 Phase 3):** additive, and always present on a
    /// successful preview — there is no state in which this run exists and has no
    /// command line, so it is a `String` rather than an `Option`.
    ///
    /// **Not [`WireCliParity`], which is a different "cli" entirely** and sits a
    /// few fields above inside `aliases`. That one asks whether a *terminal
    /// `hledger`* would produce the same accounts as this screen, and is about
    /// `hledger.conf`'s `--alias`. This one is Ledgeline's own invocation. The
    /// names are deliberately unalike so a reader is never asked to tell them
    /// apart by context.
    ///
    /// Built by [`cli_argv`], the same function `ledgeline import` is parsed
    /// into, so what this says and what that does cannot drift. Carries only the
    /// relative handles this request already used — never an absolute path — so
    /// it is run from the journal's own directory, which is what the panel says.
    cli_command: String,
    /// What matching this statement's rows against the journal by id found, or
    /// `null` when the rules file declares no id. See [`WireIdMatches`].
    ///
    /// **Contract amendment (WP-16 Phase 4):** additive and opt-in. `entries`
    /// and `count` above are already net of it — a row the journal demonstrably
    /// holds is not in the proposal this preview shows, and therefore not in the
    /// bytes the commit appends.
    id_matches: Option<WireIdMatches>,
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
    /// Whether this same import, run from a terminal, would come out the same.
    cli: WireCliParity,
}

/// Would a plain command-line `hledger import` produce these accounts too?
///
/// The question is not rhetorical. An `alias` directive in a journal is **not**
/// applied to an imported CSV — Ledgeline forwards it as `--alias`, which is the
/// only way it can reach one — so the same statement, the same rules file and the
/// same journal give a terminal `hledger import` different account names. Two
/// journals, silently, depending on which tool the user reached for.
///
/// An `hledger.conf` closes the gap because it applies to every hledger command.
/// So this reports whether one does, and offers to write one.
///
/// **`matches` is MEASURED**, on the same principle as [`WireAliasEffect`]'s
/// renames: the import is repeated with exactly the aliases a config file
/// supplies — which is exactly what a terminal would apply — and the two
/// proposals are diffed. Nothing here compares alias *strings* to decide it, so a
/// user who hand-wrote an equivalent mapping in a spelling of their own gets
/// silence rather than a lecture.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireCliParity {
    /// True when a command-line import would write the same accounts.
    matches: bool,
    /// The accounts that would differ, command-line answer → Ledgeline's.
    /// Empty when `matches`.
    differences: Vec<WireRename>,
    /// The config file in force, relative to the journal's directory — or, when
    /// it sits above that directory, `../hledger.conf` repeated per level. Never
    /// an absolute path, and `null` when there is none.
    conf_path: Option<String>,
    /// The config in force is outside the journal's own directory, so Ledgeline
    /// will **report** rather than write there. See [`resolve_conf`].
    conf_outside: bool,
    /// A command word the config forces on every hledger invocation, which makes
    /// it break every command the user runs. `null` in the ordinary case.
    conf_hijacked_by: Option<String>,
    /// The `--alias` lines the one-click fix would add, shown before it is
    /// pressed because the conversion widens what the pattern matches.
    additions: Vec<String>,
    /// Aliases that cannot be expressed in a config file at all, each with the
    /// reason. Reported, never silently dropped.
    refusals: Vec<WireConfRefusal>,
    /// Echo this in `POST /api/import/hledger-conf`. Empty string when the file
    /// does not exist yet, which is itself the revision of "no file".
    revision: String,
    /// May the fix be offered? False when the config in force is outside the
    /// journal's directory, when the journal's own directory holds something
    /// that is not a regular file, or when editing is disabled.
    writable: bool,
}

/// One alias that cannot be written into a config file, and why.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireConfRefusal {
    /// The alias as it reads in the journal.
    pattern: String,
    /// Its replacement.
    replacement: String,
    /// A closed set the UI may switch on.
    reason: &'static str,
    /// The sentence to show.
    message: &'static str,
}

/// `POST /api/import/hledger-conf` — what was written.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireConfWritten {
    /// The file, relative to the journal's own directory. Always
    /// `hledger.conf`; carried so the UI never has to spell a path itself.
    conf_path: String,
    /// The file did not exist and was created.
    created: bool,
    /// The `--alias` lines added. Empty when the config already supplied them.
    added: Vec<String>,
    /// The new revision, for a subsequent write.
    revision: String,
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

/// What this statement's rows turned out to be, matched against the journal by
/// the row id its rules file writes (`comment id:%fitid`).
///
/// **`null` when the rules file declares no id**, which is every rules file
/// written before this feature existed and is byte-for-byte today's behaviour —
/// see [`reconcile_ids`]. Never an empty object: "there is no id to match on"
/// and "there is, and nothing matched" are different answers and the UI has to
/// be able to tell them apart.
///
/// The two counts answer "how much of this statement did I already have?"; the
/// two lists carry the rows a person has to look at. See
/// [`reimport`](ledgeline_core::reimport) for what may and may not follow from a
/// match — in short, an id match may keep a row *out* of an import and may sync
/// a clearing status, and may never do anything else.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireIdMatches {
    /// Rows no transaction in the journal claims. Imported as usual — subject,
    /// still, to hledger's own `.latest` dedup.
    new: usize,
    /// Rows the journal already holds, identically. Not imported, not edited.
    unchanged: usize,
    /// Rows whose only difference is the clearing status: the authorization
    /// hold that settled. Capped at [`MAX_ID_REPORTS`]; `statusChangedTotal` is
    /// the real number.
    status_changed: Vec<WireStatusChange>,
    /// How many there are, whether or not they all fit in the list.
    status_changed_total: usize,
    /// Rows the journal holds *differently* in some way a status flip cannot
    /// express. Never imported and **never edited** — this is the hand-edit the
    /// feature exists to protect. Capped at [`MAX_ID_REPORTS`].
    conflicting: Vec<WireConflict>,
    /// How many there are, whether or not they all fit in the list.
    conflicting_total: usize,
}

/// One clearing status this statement moved.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireStatusChange {
    /// The row id, as the rules file wrote it.
    id: String,
    /// What the journal says today: `unmarked`, `pending` or `cleared`.
    from: &'static str,
    /// What this statement says.
    to: &'static str,
    /// Whether it was actually written.
    ///
    /// Always `false` on a **dry-run**, which previews and writes nothing — the
    /// same rule the rest of this screen follows. On a **commit** it is `false`
    /// only for a transaction that lives outside the file this import writes to:
    /// the flip is confined to `journalId`, the one file whose git state was
    /// checked before the commit and whose new bytes the commit's own git commit
    /// carries. Syncing a status into some other included file would write
    /// somewhere this request had neither permission for nor a way to undo.
    applied: bool,
}

/// One row the journal already holds differently.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireConflict {
    /// The row id, as the rules file wrote it.
    id: String,
    /// Every disagreement, in field order. Capped at [`MAX_ID_DIFFS`].
    diffs: Vec<WireFieldDiff>,
}

/// One field a conflicting row disagrees on.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireFieldDiff {
    /// What disagrees: `date`, `description`, `posting 2 amount`, …
    field: String,
    /// What the journal says today.
    existing: String,
    /// What this statement proposes.
    incoming: String,
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
    /// What matching this statement's rows against the journal by id found, or
    /// `null` when the rules file declares no id. See [`WireIdMatches`]. The
    /// same type the dry-run carries, so one decoder serves both; here its
    /// `statusChanged[].applied` reports what was actually written.
    id_matches: Option<WireIdMatches>,
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
    /// What the git safety net did with the re-sorted journal, on the same terms
    /// as [`WireCommit::git`]. `null` when the journal is not under version
    /// control, when autocommit is off, or when nothing moved.
    git: Option<WireGitResult>,
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

/// The `hledger-conf` body — **one field, and it is not the content**.
///
/// The client says only *which bytes it planned against*; what to write is
/// recomputed here from the journal's own `alias` directives. That is security
/// layer 4 (content provenance) applied to a new write target: a body carrying
/// the lines to write would make this route a write-arbitrary-text primitive
/// aimed at a file hledger executes options out of, which is a considerably worse
/// thing to own than the rules editor's.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireConfRequest {
    /// The revision the caller read. The empty string means "there was no file".
    revision: String,
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

/// [`plain_field`], and additionally shaped like an account name rather than an
/// option, a comment, or two fields.
///
/// For the balance ACCOUNT, which plays two roles and is checked for both. It
/// becomes a positional query argument to `hledger balance`, and it is also
/// written verbatim into the posting line [`assertion_lines`] appends to the
/// journal — so anything that changes where hledger thinks the NAME ends is a
/// correctness problem, not tidiness.
///
/// 1. **A leading `-`.** `Invocation` passes arguments as a `Vec<OsString>` with
///    no shell in sight, so there is nothing to quote — but a value beginning
///    with `-` is still read by hledger's own parser as a flag, and there is no
///    `--` terminator on a query.
/// 2. **A `;`.** There is no end-of-line comment *inside* an account name: the
///    name runs to the two-space separator, so `assets:bank ; note` is one
///    account whose name contains a semicolon. Verified against hledger 1.52 —
///    `hledger accounts` reports `assets:bank ; note` alongside `assets:bank`.
///    This is the same defect `rules::account_comment_warning` exists for,
///    arriving through a different door, and it is not hypothetical: it has
///    already put real transactions into an account named for its own comment.
///    It can even pass the [`check_assertion`] gate — a phantom account has a
///    zero balance, so asserting `$0` against it succeeds and the line lands.
/// 3. **Two spaces in a row.** That *is* the separator between an account name
///    and its amount on a posting line, so `assets:bank  checking` would be read
///    as the account `assets:bank` with an amount of `checking`. hledger refuses
///    the result outright, which at least fails closed, but it fails with a
///    parse error about an unexpected `$` that says nothing about the real
///    cause.
///
/// **`#` is deliberately NOT refused**, for the reason it is not flagged in a
/// rules file either: `assets:card #1234` is a plausible account name. The
/// reasoning is in fact *stronger* here. `#` opens a comment in a journal only
/// at the very start of a line, and [`assertion_lines`] always writes the
/// account indented by four spaces, so it can never land in that column.
/// Confirmed against hledger 1.52: `assets:card #1234` round-trips through
/// `hledger accounts` intact.
///
/// No account name can contain any of the three refused shapes — hledger cannot
/// represent them — so refusing them costs nothing.
fn argument_field(value: &str, what: &str) -> Result<String, AppError> {
    let field = plain_field(value, what)?;
    if field.starts_with('-') {
        return Err(AppError::BadRequest(format!(
            "{} is not a usable {what}: it may not begin with `-`, which hledger would read as an \
             option rather than a name",
            quoted(value)
        )));
    }
    if field.contains(';') {
        return Err(AppError::BadRequest(format!(
            "{} is not a usable {what}: it may not contain `;`. An account name has no end-of-line \
             comment — it runs to the end of the field — so the `;` would not start one. The \
             journal would gain a real account whose NAME is that entire string, semicolon and \
             note included",
            quoted(value)
        )));
    }
    if field.contains("  ") {
        return Err(AppError::BadRequest(format!(
            "{} is not a usable {what}: it may not contain two spaces in a row, which is what \
             separates an account name from its amount, so hledger would stop reading the name at \
             them and try to take the rest as a number",
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

/// What a rules file's own `skip` says, or `0` when it says nothing.
///
/// The number every copy of the staged CSV hledger reads has to be aligned to —
/// see [`convert::align_to_skip`], and [`Plan::skip`] for where it is kept.
/// `skip` is **first-one-wins**, unlike almost every other directive, and
/// `RulesDoc::settings` already knows that; reading the raw text for a `skip`
/// line here instead would get it wrong on the file that says it twice.
///
/// An unreadable file answers `0`, which means no padding, which is exactly
/// today's behaviour — and the import is about to fail on its own terms anyway,
/// with hledger's message rather than a guess of ours.
fn rules_skip(rules: &Path) -> u32 {
    std::fs::read_to_string(rules)
        .ok()
        .and_then(|text| RulesDoc::parse(&text).settings().skip)
        .map_or(0, |setting| setting.value)
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
// hledger.conf — the second home an account alias can live in
// ===========================================================================

/// The `hledger.conf` in force for this journal, if there is one.
///
/// "In force" is hledger's own answer, narrowed: it searches the working
/// directory and every directory above it, and Ledgeline searches upward from the
/// **journal's** directory instead, because a server process's working directory
/// is an accident of how it was launched while the journal's directory is where a
/// user runs hledger for these books. `$HOME/.hledger.conf` and the XDG config
/// dir, which hledger falls back to, are deliberately not consulted — see
/// [`hledger_conf::locate`].
struct ConfInForce {
    /// A relative handle for the wire: `hledger.conf`, or `../hledger.conf`
    /// repeated per level when it sits above the journal's directory.
    id: String,
    /// It is above the journal's own directory, so Ledgeline reads it and will
    /// not write it.
    outside: bool,
    /// The `--alias` values it gives an import, general section then `[import]`.
    aliases: Vec<String>,
    /// A command word it forces on every hledger invocation, if it forces one.
    hijacked_by: Option<String>,
}

/// Find and read the config file in force, if any. Never an error: a config file
/// that cannot be read is a config file we do not have.
fn conf_in_force(root: &Path) -> Option<ConfInForce> {
    let path = hledger_conf::locate(root)?;
    let text = hledger_conf::read(&path).ok()?;
    let directory = path.parent()?;
    let outside = directory != root;
    Some(ConfInForce {
        id: conf_id(root, directory),
        outside,
        aliases: hledger_conf::alias_arguments(&text, hledger_conf::IMPORT_COMMAND),
        hijacked_by: hledger_conf::hijacks_command(&text),
    })
}

/// A relative handle for a config file's location, from the journal's directory.
///
/// Security layer 5: the client is told `../hledger.conf`, never
/// `/Users/someone/hledger.conf`. The number of `../` levels is real information
/// — it is how the user finds the file — and it discloses nothing a relative
/// rules id does not.
fn conf_id(root: &Path, directory: &Path) -> String {
    let levels = root
        .strip_prefix(directory)
        .map_or(1, |rest| rest.components().count());
    format!("{}{}", "../".repeat(levels), hledger_conf::CONF_NAME)
}

/// The config file this server may write: `hledger.conf` in the journal's **own**
/// directory, and nowhere else.
///
/// # Why the location is not negotiable
///
/// This is a new write target and it gets the same discipline as every other one
/// here, which for a location means: the journal's directory is the tree the user
/// pointed us at, so it is the whole of the tree we write in.
///
/// **`$HOME/.hledger.conf` and the XDG config dir are never written**, though
/// hledger would read either. They are outside that tree, they affect every set
/// of books on the machine rather than these ones, and a desktop application
/// quietly editing a home-directory dotfile is not a thing to do. A config in
/// force above the journal's directory is likewise reported and not touched.
///
/// # The guards, in order
///
/// 1. The path is built from the include root and one fixed file name — there is
///    no client-supplied component anywhere in it, so layers 1 and 2 have nothing
///    to check.
/// 2. [`parse::confine`], the same containment `include` and the rules scan use,
///    applied to the parent. It canonicalizes first, so a journal directory that
///    is itself reached through a symlink is resolved before the comparison.
/// 3. [`std::fs::symlink_metadata`], which does not follow links: the target must
///    be absent or a **regular file**. A symlinked `hledger.conf` pointing at
///    `~/.bashrc` is refused, and so is a FIFO, which would otherwise hang the
///    request forever on `read`.
struct ConfTarget {
    path: PathBuf,
    /// The wire handle. Always `hledger.conf`.
    id: String,
    text: String,
    exists: bool,
    /// A fingerprint of the bytes, or `""` for a file that does not exist. The
    /// empty string is a real revision — "there was nothing here" — and a write
    /// that echoes it is refused if a file has appeared since.
    revision: String,
    writable: bool,
}

fn resolve_conf(root: &Path) -> ConfTarget {
    let absent = |writable: bool| ConfTarget {
        path: root.join(hledger_conf::CONF_NAME),
        id: hledger_conf::CONF_NAME.to_string(),
        text: String::new(),
        exists: false,
        revision: String::new(),
        writable,
    };
    // Guard 2. The parent is the root itself, so this is cheap — and it is here
    // because "cheap and obviously true today" is how a containment check stops
    // being performed at all.
    let Some(directory) = ledgeline_core::parse::confine(root, root) else {
        return absent(false);
    };
    let path = directory.join(hledger_conf::CONF_NAME);
    // Guard 3.
    match std::fs::symlink_metadata(&path) {
        Err(_) => absent(true),
        Ok(meta) if meta.file_type().is_file() => match hledger_conf::read(&path) {
            Ok(text) => ConfTarget {
                revision: Fingerprint::of_bytes(text.as_bytes()).token(),
                id: hledger_conf::CONF_NAME.to_string(),
                exists: true,
                writable: true,
                text,
                path,
            },
            Err(_) => absent(false),
        },
        // A symlink, a directory, a FIFO: present, and not something to write.
        Ok(_) => ConfTarget {
            writable: false,
            exists: true,
            ..absent(false)
        },
    }
}

/// Where every `--alias` on an import's command line came from.
///
/// Two lists rather than one because the divergence notice needs to tell them
/// apart: `conf` is what a terminal would apply, and `merged` is what this import
/// is given. The difference between them is the divergence.
#[derive(Debug, Default, Clone)]
struct AliasArguments {
    /// From `hledger.conf`. Exactly what a plain command-line hledger applies.
    conf: Vec<String>,
    /// What this import is given: `conf`, then the journal's own aliases.
    merged: Vec<String>,
}

impl AliasArguments {
    /// Merge the two sources.
    ///
    /// **The config's aliases go first, and the order is the point.** `--alias`
    /// options compose left to right and the first one to match an account is the
    /// one that rewrites it, so putting the config first means that wherever the
    /// two disagree about an account, the config wins — and the config is what a
    /// terminal `hledger import` would have applied. Journal-first would make
    /// Ledgeline disagree with the command line in a *second*, opposite way,
    /// which is precisely the thing this feature exists to remove.
    fn merge(journal: Vec<String>, conf: Vec<String>) -> Self {
        let merged = conf
            .iter()
            .cloned()
            .chain(
                journal
                    .iter()
                    .filter(|argument| !conf.contains(argument))
                    .cloned(),
            )
            .collect();
        Self { conf, merged }
    }

    /// Is what a terminal would apply already everything this import applies?
    /// When so, no measurement is needed: the two command lines are equal.
    fn identical_to_conf(&self) -> bool {
        self.merged == self.conf
    }
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

/// What one `hledger import` run is for.
///
/// **There is no variant that writes**, and that is the point of the type.
/// hledger never appends to a user's journal: it proposes (`--dry-run`),
/// Ledgeline appends that exact text through
/// [`edit::atomic_write`](ledgeline_core::edit::atomic_write), and then hledger
/// is asked to record its own dedup state (`--catchup`). A third variant
/// spelling "and now actually write it" would put back the split between what
/// the preview showed and what landed — see [`run_commit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImportRun {
    /// `--dry-run`: the proposed entries on stdout, nothing on disk.
    Preview,
    /// `--catchup`: `.latest.NAME` recorded, nothing appended. Verified against
    /// hledger 1.52 — the journal is byte-identical afterwards, the state file
    /// is byte-identical to the one a real import writes (repeated same-date
    /// lines and all), and a following dry-run reports no new transactions.
    Catchup,
}

impl ImportRun {
    /// The one flag that distinguishes them.
    const fn flag(self) -> &'static str {
        match self {
            Self::Preview => "--dry-run",
            Self::Catchup => "--catchup",
        }
    }
}

/// `hledger [--alias=…]… -I -f JOURNAL import (--dry-run|--catchup) --rules RULES CSV`.
///
/// Every path is absolute, which is also what makes them safe as positional
/// arguments: an absolute path begins with `/` and can never be read as an
/// option, so no `--` terminator is needed to protect a statement named `-f`.
///
/// `--rules`, not `--rules-file`: the flag was renamed in hledger 1.40, which is
/// why [`MIN_HLEDGER`](crate::hledger::MIN_HLEDGER) is 1.40.
///
/// # Why `--ignore-assertions` is here, and must stay
///
/// **DO NOT REMOVE THIS FLAG TO "RESTORE SAFETY". It restores nothing.**
///
/// `import` is the one invocation that reads the *target file on its own*: one
/// `-f`, naming the fragment the new entries are appended to. A balance
/// assertion inside an included fragment is not a safety check we are choosing
/// to switch off — it is a check that is **structurally incapable of being
/// correct in that context**, because the balance it asserts accumulates
/// through files hledger was never asked to read. Evaluating it there produces
/// a false failure, not safety.
///
/// The split-year layout is the ordinary case, not an exotic one. `main.journal`
/// includes `2025/2025.journal` then `2026/2026.journal`, and the 2026 file opens
/// with a start-of-year assertion carrying 2025's closing balance. Verified
/// against hledger 1.52:
///
/// ```text
/// $ hledger -f main.journal check                      # the tree is fine
/// $ hledger -f 2026/2026.journal import --rules … bank.csv
/// hledger: Error: …/2026/2026.journal:3:31:
///   3 |     assets:bank:checking              $0 = $900.00
/// Balance assertion failed in assets:bank:checking
///   the asserted balance is:   $900.00
///   but the calculated balance is:   $0
/// ```
///
/// The import aborts on a journal that is correct. The place that assertion is
/// meaningful is the root, where it is still evaluated and still protects the
/// user — by `hledger check`, by every report Ledgeline renders, and by
/// [`verify_balance`] and [`check_assertion`], both of which read the **root**
/// for exactly this reason.
///
/// Two things the flag does *not* cost, both verified against 1.52 rather than
/// assumed:
///
/// * **Assertions a rules file GENERATES are still written, and still checked.**
///   A `balance` field emits `assets:bank:checking  $-20.00 = $880.00` into the
///   proposed entries, `-I` leaves that text exactly as it is, and it lands in
///   the journal to be checked at the root from then on. Deferred, not lost.
/// * **Nothing is being skipped that this invocation ever checked.** hledger does
///   not evaluate CSV-derived assertions during an import at all: importing a
///   `balance`-field CSV asserting `$880.00` into a journal holding `$100.00`
///   exits zero. The only assertions `-I` suppresses are the target fragment's
///   own — the ones that cannot hold when read in isolation.
///
/// # The run kind is a parameter, and that is the point
///
/// The preview, the dedup measurement, the alias measurements and the catch-up
/// share **one** argv builder, so there is no way for them to be given different
/// aliases or a different target. That matters more than it reads: a preview
/// that showed `assets:morganstanley:pw-roth-ira` while the commit wrote
/// `PW Roth IRA - 3077` would be a lie told immediately before the only
/// irreversible step on the screen. Making them agree by construction is
/// stronger than a test asserting that they do — though
/// `import_endpoints.rs::a_dry_run_and_a_commit_agree_on_aliased_accounts` also
/// asserts it, against the real binary.
///
/// It matters twice over now that the commit runs a **preview** and appends the
/// result itself: the bytes the user approved and the bytes that land come from
/// the same command line, differing only in the `--dry-run`/`--catchup` word.
fn import_invocation(
    hledger: &Hledger,
    journal: &Path,
    rules: &Path,
    csv: &Path,
    aliases: &[String],
    run: ImportRun,
) -> Invocation {
    hledger
        .invoke(alias_flags(aliases))
        // Before the subcommand, for the same reason `--no-conf` is: this is a
        // general option, and hledger's own parser reads them in order.
        .arg(IGNORE_ASSERTIONS)
        .args(["-f".as_ref(), journal.as_os_str()])
        .arg("import")
        .arg(run.flag())
        .arg("--rules")
        .arg(rules)
        .arg(csv)
        .timeout(IMPORT_TIMEOUT)
}

/// The bytes to append to a journal for `entries`, exactly as `hledger import`
/// would have appended them.
///
/// **Compared against the real thing, byte for byte** (hledger 1.52). A dry-run
/// writes its proposal to stdout as the transactions followed by a **blank
/// line**; a real import appends a leading `\n` and the same text with that
/// blank line removed:
///
/// ```text
/// stdout   "2026-02-01 A\n    …$-405\n\n2026-02-03 B\n    …$165.2\n\n"
/// appended "\n2026-02-01 A\n    …$-405\n\n2026-02-03 B\n    …$165.2\n"
/// ```
///
/// Note what hledger does *not* do: it does not check whether the file already
/// ends in a newline. A journal saved without a trailing newline gets the first
/// imported transaction on the line straight after the last posting — verified,
/// and still valid hledger, since a transaction begins at column 1. Reproducing
/// that exactly is deliberate: matching hledger's own output was the cheapest
/// way to be sure this change moved no bytes it was not asked to.
///
/// An empty proposal appends **nothing at all**, which is also what hledger does
/// with a statement holding no new rows.
fn appended_text(entries: &str) -> String {
    let body = entries.trim_end_matches('\n');
    if body.trim().is_empty() {
        String::new()
    } else {
        format!("\n{body}\n")
    }
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

/// Verify a statement balance over **the whole journal tree + the proposed
/// entries**, as one journal.
///
/// # Why the ROOT and not the file being written
///
/// `root` is the journal Ledgeline was opened with — the one that `include`s
/// everything — and it is deliberately **not** the import's target. The two are
/// different journals and only one of them can answer this question.
///
/// A statement balance is a claim about an account, and an account's balance
/// accumulates across every file in the tree. In the split-year layout, with
/// `main.journal` including `2025/2025.journal` and `2026/2026.journal`, the
/// checking balance after a $5 coffee is $895.00 at the root and $-5.00 in the
/// 2026 fragment alone. Reading the target gave the user the second number and
/// told them their statement did not match — a silent wrong answer, and one
/// that also refused correct balances through [`check_assertion`].
///
/// `include <root>` + the proposed entries is exactly *what the tree will look
/// like once this import lands*, with no double counting: the proposed entries
/// are hledger's dry-run stdout, which by definition is not in the target yet,
/// and the target is reached through the root's own `include`. After the import
/// has been applied [`write_assertion`] passes an empty `proposed` for the same
/// reason — the entries are in the target by then, and the root sees them there.
///
/// **No `-I` here, and that is the point.** At the root an assertion is
/// evaluated in the context it was written for, so it is meaningful: a failure
/// is real information and answers `None` ("not known") below. Only
/// [`import_invocation`], which reads a fragment on its own, may disable them.
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
    root: &Path,
    proposed: &str,
    account: &str,
) -> Result<Option<String>, AppError> {
    let output = run(
        hledger
            .invoke(["-f", "-", "balance"])
            .arg(account)
            .args(["--no-total", "--flat", "-O", "csv"])
            .stdin(concatenated(root, proposed).into_bytes())
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

/// The single journal handed to hledger on stdin: the **root** journal by
/// absolute `include`, then the proposed entries. See [`verify_balance`].
fn concatenated(root: &Path, proposed: &str) -> String {
    format!("include {}\n\n{proposed}\n", root.display())
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
/// looking like two different numbers. **Below [`MAX_RENDER_PLACES`] it never
/// truncates**: a `places` under the value's own scale keeps the value's,
/// because dropping a digit to make two numbers look alike is how a 5-cent gap
/// disappears from a screen.
///
/// # Total by construction
///
/// Both the requested `places` **and** the value's own scale are clamped to
/// [`MAX_RENDER_PLACES`], because both feed a `"0".repeat(…)`: either one left
/// as a bare `u32` turns a short request into a proportional allocation. That is
/// the defect `convert::ofx` was carrying, where a 345-byte statement rendered
/// to 20 MB, and this was the worse instance of it — two unbounded repeats
/// rather than one. The bound is the engine's single copy, shared with
/// `edit::render_dec` and `assertions::render_dec`; it is imported, never
/// restated.
///
/// Past the bound a render is a **different number**, which is the same trade
/// those two renderers make and the reason the clamp is documented rather than
/// silent. Nothing on this route can reach it today — every [`Dec`] here comes
/// from `Dec::parse`, which caps scale at `MAX_PARSE_PLACES` — but a renderer is
/// not entitled to assume its caller validated anything, and this one is called
/// with a scale derived from *two* values rather than one.
fn render_money_at(value: Dec, places: u32) -> String {
    let scale = value.places.min(MAX_RENDER_PLACES);
    let wanted = places.min(MAX_RENDER_PLACES);
    let pad = usize::try_from(wanted.saturating_sub(scale)).unwrap_or(0);
    let places = usize::try_from(scale.max(wanted)).unwrap_or(0);
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

/// Whether THIS run may use the git safety net at all, before the preferences
/// are even consulted.
///
/// Every HTTP caller passes [`FromPrefs`](Self::FromPrefs), which is exactly the
/// behaviour that existed before this type: the stored preference decides. It is
/// a parameter rather than another read of `prefs::load()` because
/// `ledgeline import --no-git` has to turn the net off for **one invocation**
/// without writing anything into a store that outlives it — a CLI flag that
/// silently edited the desktop app's preferences would be a considerably worse
/// bargain than the one the flag offers.
///
/// Both halves of the net move together, deliberately: with [`Off`](Self::Off)
/// a dirty target no longer blocks the commit AND nothing is committed
/// afterwards. Suppressing only the second would leave the refusal in place
/// with no safety left to justify it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GitPolicy {
    /// Ask the preferences store, as every `/api/import/*` request does.
    FromPrefs,
    /// Off for this run only, whatever the preferences say (`--no-git`).
    Off,
}

impl GitPolicy {
    /// Is the safety net live for this run?
    fn enabled(self, prefs: &Prefs) -> bool {
        self == Self::FromPrefs && autocommit_enabled(prefs)
    }
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
            // What came back is what was STAGED, which is what went in minus
            // whatever the repository ignores — a `.latest.*` or a statements
            // directory the user deliberately keeps out of version control. The
            // rest is `skipped`, which is what that field has always promised;
            // reporting them all as committed would be a claim `git log`
            // contradicts, and it stopped being hypothetical when the dedup
            // marker joined this set.
            Ok(staged) => {
                let was_staged =
                    |path: &Path| repo.relative(path).is_ok_and(|name| staged.contains(&name));
                committed.extend(
                    group
                        .iter()
                        .filter(|(path, _)| was_staged(path))
                        .map(|(_, handle)| (*handle).to_string()),
                );
                skipped.extend(
                    group
                        .iter()
                        .filter(|(path, _)| !was_staged(path))
                        .map(|(_, handle)| (*handle).to_string()),
                );
            }
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
    SourceFormat::ALL
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

    let (id, staged) = state.stages().put(&csv, format, name).map_err(|error| {
        AppError::Internal(format!("could not stage this upload: {}", error.kind()))
    })?;

    // Candidate scoring needs the journal's own directory to scan for rules
    // files. Without a journal there is nothing to score against — which is not
    // an error, just an empty list and a default derived from the upload's name.
    let main = state.source_files().into_iter().next();
    // The same merged set the dry-run will use — both homes, config first — so a
    // candidate card's sample accounts are the accounts the import proposes. Two
    // different alias sets between the card and the preview would make the card
    // a lie about the very thing this feature exists to make visible.
    let aliases = main
        .as_deref()
        .and_then(|main| include_root(main).ok())
        .map(|root| {
            AliasArguments::merge(
                aliases::arguments(&state.snapshot().journal),
                conf_in_force(&root)
                    .map(|conf| conf.aliases)
                    .unwrap_or_default(),
            )
            .merged
        })
        .unwrap_or_default();
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
    //
    // Aligned to `skip 0`, i.e. not padded, and that is safe for one reason
    // only: no rules file has been CHOSEN yet, and every route that hands this
    // slot to hledger (`run_dry_run`, `preflight_assertion`) re-materialises it
    // first with the plan's own `skip`. The `.latest` copy beside it is the
    // whole point of running this early; the CSV is a placeholder.
    if let Some(root) = main.as_deref().map(include_root).transpose()?
        && let Ok(destination) = resolve_destination(&root, &defaults.csv_path)
        && let Some((dir, file)) = destination.parent().zip(file_name(&destination))
    {
        let _ = staged.materialize(RUN_WITH_LATEST, &file, Some(dir), 0);
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
    // Each candidate's own `skip` is carried alongside its prefilter result,
    // because the file it is scored against has to be padded into ITS frame.
    // Scored against a header-on-line-1 CSV, a genuine `skip 3` file reads the
    // statement's tail and nothing else — which is not even a low score: it is a
    // clean 1.0 for correctly importing a third of the file, and a zero only
    // when nothing at all is left. See `convert::align_to_skip`.
    let mut survivors: Vec<(usize, matching::PrefilterPass, u32)> = discovery
        .files
        .iter()
        .take(MAX_PREFILTERED)
        .enumerate()
        .filter(|(_, found)| found.parsed)
        .filter_map(|(at, found)| {
            let text = std::fs::read_to_string(found.path().as_path()).ok()?;
            let doc = RulesDoc::parse(&text);
            let skip = doc.settings().skip.map_or(0, |setting| setting.value);
            let pass = matching::prefilter(&doc, data)?;
            Some((at, pass, skip))
        })
        .collect();
    survivors.sort_by(|(a, ..), (b, ..)| {
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
        .filter_map(|(at, pass, skip)| {
            let found = &discovery.files[at];
            let data = staged.aligned(skip).ok()?;
            let json = print_json(hledger, &data, found.path().as_path(), aliases)?;
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
///
/// # And no `--ignore-assertions`, unlike [`import_invocation`]
///
/// This reads the CSV and no journal at all, so there is no fragment here whose
/// assertions could be evaluated out of context. The only assertions in reach
/// would be ones the *rules file* generates from a `balance` field — and
/// hledger does not evaluate those when reading a CSV. Verified against 1.52: a
/// rules file whose `balance` column asserts `$880.00` prints
/// `assets:bank:checking  $-20.00 = $880.00` and exits **zero** from a running
/// total of `$-20.00`. There is nothing here for the flag to suppress, so it is
/// not passed; a candidate is never lost to an assertion.
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
    let Json(body) = compute(move || run_dry_run(&state, &request, GitPolicy::FromPrefs)).await?;
    Ok(no_store(body))
}

/// Everything a dry-run and a commit both have to resolve, resolved once.
///
/// # Two journals, and neither may be called `journal`
///
/// This type used to hold one field named `journal`, meaning the file being
/// written to, while most of what read it wanted the *other* journal — and that
/// single overloaded name is precisely how the balance verification came to
/// reckon against a fragment. So the two are named for the jobs they do and
/// neither gets the bare word:
///
/// * [`target`](Self::target) — the file this import WRITES to. Appending is the
///   only thing it is for.
/// * [`root_journal`](Self::root_journal) — the file the tree is RECKONED
///   AGAINST: the journal Ledgeline was opened with, which `include`s the target
///   and everything else. Every balance question goes here.
///
/// A field named `journal` would be a question ("which one?") answered by
/// whoever typed the next line fastest. There is no such field on purpose;
/// resist adding one back.
struct Plan {
    hledger: Hledger,
    staged: std::sync::Arc<Stage>,
    /// The file this import appends to — the WRITE destination, and nothing
    /// else. It is a fragment of a larger journal in every layout but the
    /// single-file one, so it is never the thing to compute a balance over.
    target: PathBuf,
    /// The journal Ledgeline was opened with: the root of the `include` tree the
    /// target is part of. This is what balances and assertions are reckoned
    /// against — see [`verify_balance`].
    root_journal: PathBuf,
    rules: PathBuf,
    /// The chosen rules file's own `skip`, read from the file at resolve time.
    ///
    /// It is a number the user wrote against their **raw download**, and the
    /// conversion moved the header out from under it, so every copy of the CSV
    /// hledger reads on this plan is padded back into that frame —
    /// [`convert::align_to_skip`] argues why, and why the alternative is a
    /// silent import of nothing. Resolved once, here, for the same reason
    /// [`aliases`](Self::aliases) is: the dry-run and the commit must not be
    /// able to disagree about it.
    skip: u32,
    destination: PathBuf,
    /// The destination's bare file name — what the staged copy is named, and what
    /// `.latest.NAME` is keyed to.
    csv_name: String,
    /// Every `--alias` this import gets, and where each came from.
    ///
    /// Resolved once, here, so the dry-run and the commit cannot be handed
    /// different sets — see [`import_invocation`].
    aliases: AliasArguments,
    /// Every `alias` the journal declares, forwarded or refused — the input the
    /// one-click config fix is computed from. Kept beside the arguments because
    /// writing a config line needs the pattern and replacement apart, which an
    /// assembled `OLD=NEW` argument has already joined.
    declared: Vec<aliases::Forwarded>,
    /// The journal's own DIRECTORY — the include root every handle is confined
    /// to. Named for the thing it is, because a `root` beside a `root_journal`
    /// is the ambiguity this type's docs are about.
    root_dir: PathBuf,
    /// The `hledger.conf` in force at or above [`root_dir`](Self::root_dir).
    conf: Option<ConfInForce>,
    redactor: Redactor,
}

impl Plan {
    /// Resolve every handle in `request`, in the order that puts the cheapest
    /// refusals first.
    fn resolve(state: &AppState, request: &WireDryRunRequest) -> Result<Self, AppError> {
        let staged = resolve_stage(state, &request.stage_id)?;
        // The journal this server was opened with. It is the root of the
        // `include` tree — `journals::targets` flags it `is_root` — so it is
        // both the directory anchor and, as a file, the thing every balance is
        // reckoned against. Two different uses, two named fields below.
        let root_journal = main_journal(state, "rules file", &request.rules_id)?;
        let root_dir = include_root(&root_journal)?;
        let (target, _) = resolve_journal(state, &root_dir, &request.journal_id)?;
        let rules = resolve_rules(&rules::discover(&root_journal), &request.rules_id)?;
        let skip = rules_skip(&rules);
        let destination = resolve_destination(&root_dir, &request.csv_path)?;
        let csv_name = file_name(&destination)
            .ok_or_else(|| unresolved("CSV destination", &request.csv_path))?;
        // Sequencing rule 1, second half. Resolved here for the reason `skip`
        // and `aliases` are: the dry-run and the commit must not be able to
        // disagree about whether the dedup state applies.
        refuse_unusable_dedup_state(&destination, &csv_name, &request.csv_path)?;
        let hledger = resolve_hledger()?;
        // Both homes an account alias can live in — see `AliasArguments::merge`
        // for why the config's come first.
        let conf = conf_in_force(&root_dir);
        let declared = aliases::forward(&state.snapshot().journal);
        let aliases = AliasArguments::merge(
            declared
                .iter()
                .filter_map(|alias| alias.argument().map(str::to_string))
                .collect(),
            conf.as_ref()
                .map(|conf| conf.aliases.clone())
                .unwrap_or_default(),
        );

        let redactor = Redactor::default()
            // hledger echoes its own argv[0] in a usage dump — which is what an
            // unrecognised flag produces — and under Nix that is a store path.
            .hide(hledger.path(), "hledger")
            .hide(&target, &request.journal_id)
            .hide(&rules, &request.rules_id)
            .hide(&destination, &request.csv_path)
            // Covers `root_journal` too: it lives in this directory, and the
            // balance verifications put its absolute path into hledger's stdin,
            // so hledger's own diagnostics quote it back.
            .hide_prefix(&root_dir)
            .hide_prefix(&std::env::temp_dir());
        Ok(Self {
            hledger,
            staged,
            target,
            root_journal,
            rules,
            skip,
            destination,
            csv_name,
            aliases,
            declared,
            root_dir,
            conf,
            redactor,
        })
    }

    /// The destination's directory — where `.latest.NAME` lives.
    fn destination_dir(&self) -> &Path {
        self.destination.parent().unwrap_or_else(|| Path::new("."))
    }

    /// The root journal's own handle, relative to [`root_dir`](Self::root_dir).
    ///
    /// It is by construction a file IN that directory (the directory is defined
    /// as its parent), so the handle is its file name — the same string
    /// [`journals::targets`] derives for it. Only [`cli_argv`] needs it, to
    /// decide whether `--root-journal` has to be said at all.
    fn root_journal_id(&self) -> Option<&str> {
        self.root_journal.file_name().and_then(|name| name.to_str())
    }

    /// The dedup marker `hledger import --catchup` maintains beside the
    /// destination, with the relative handle to report it by — or `None` when
    /// there is no file there yet.
    ///
    /// Existence is checked, and that is not defensive: `git add` on a path that
    /// does not exist fails the **whole** invocation, so a first import — which
    /// has no marker until the catch-up runs — would take the journal and the
    /// CSV down with it.
    fn marker_target(&self, csv_path: &str) -> Option<(PathBuf, String)> {
        let name = stage::latest_name(&self.csv_name);
        let path = self.destination_dir().join(&name);
        path.is_file().then(|| {
            let handle = match csv_path.rsplit_once('/') {
                Some((parent, _)) => format!("{parent}/{name}"),
                None => name.clone(),
            };
            (path, handle)
        })
    }

    /// The two targets an import **writes**, each with the handle to report it
    /// by — and the two it blocks on.
    ///
    /// These are the files that hold a user's own work, so this is the set
    /// `blocked_by_git` refuses a dirty member of. The dedup marker is written
    /// too but is not in here: it is hledger's bookkeeping rather than anybody's
    /// work, and it joins the commit set separately in [`run_commit`]. Widening
    /// this to include it would let a marker left dirty by a terminal
    /// `hledger import` block every import through the screen.
    fn targets<'a>(&'a self, request: &'a WireDryRunRequest) -> Vec<(&'a Path, &'a str)> {
        vec![
            (self.destination.as_path(), request.csv_path.as_str()),
            (self.target.as_path(), request.journal_id.as_str()),
        ]
    }
}

/// The whole of `dry-run`, synchronously.
fn run_dry_run(
    state: &AppState,
    request: &WireDryRunRequest,
    git: GitPolicy,
) -> Result<WireDryRun, AppError> {
    let plan = Plan::resolve(state, request)?;
    let staged = plan
        .staged
        .materialize(
            RUN_WITH_LATEST,
            &plan.csv_name,
            Some(plan.destination_dir()),
            plan.skip,
        )
        .map_err(stage_failed)?;

    let output = run(
        import_invocation(
            &plan.hledger,
            &plan.target,
            &plan.rules,
            &staged,
            &plan.aliases.merged,
            ImportRun::Preview,
        ),
        "run the import preview",
    )?;
    if !output.success() {
        return Ok(WireDryRun::Failed(WireDryRunFailed {
            ok: false,
            stderr: plan.redactor.apply(&output.stderr_lossy()),
        }));
    }

    // hledger's own proposal, verbatim — and therefore literally the bytes
    // `commit` will append. Nothing re-renders it in between; see `run_commit`,
    // and `docs/imports.md` § "Commodity style" for why re-printing it under the
    // tree's `commodity` directives is deliberately NOT done.
    let proposal = output.stdout_lossy();
    let status = output.stderr_lossy();

    // One extra `--dry-run`, and only when the destination carries a `.latest`.
    // Two things read it: what dedup would drop, and — when the rules file names
    // a row id — what those dropped rows actually ARE.
    let bare = bare_proposal(&plan)?;
    let ids = reconcile_ids(state, &plan, &proposal, bare.as_ref());
    // `entries` is what the commit appends, so it is what the preview shows: a
    // row the journal demonstrably already holds is taken out of both, together.
    // With no id in the rules file `ids` is `None` and this is `proposal` itself.
    let (entries, count) = match &ids {
        Some(ids) => (ids.entries.as_str(), ids.count),
        None => (
            proposal.as_str(),
            count_transactions(&proposal)
                .or_else(|| reported_count(&status, "would import"))
                .unwrap_or(0),
        ),
    };

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
            // The ROOT, not the file being written: the balance the user is
            // reconciling against their statement is the tree's, and in a split
            // layout the target holds only part of it. See `verify_balance`.
            let computed = verify_balance(&plan.hledger, &plan.root_journal, entries, &account)?;
            Ok::<_, AppError>(reconcile(&statement, computed))
        })
        .transpose()?;

    Ok(WireDryRun::Proposed(Box::new(WireProposal {
        ok: true,
        entries: plan.redactor.apply(entries),
        count,
        status: plan.redactor.apply(&status),
        skipped: skipped_by_dedup(count, bare.as_ref()),
        balance,
        // The UNFILTERED proposal, deliberately. This measures the aliases by
        // diffing two runs posting-for-posting, and both baselines are whole
        // proposals; handing it a filtered one would be a shape mismatch and the
        // renames would silently vanish. What an alias rewrites is a property of
        // the rules file, not of which rows are new.
        aliases: alias_effect(&plan, &staged, &proposal)?,
        blocked_by_git: if git.enabled(&prefs::load()) {
            blocked_by_git(&plan.targets(request))
        } else {
            Vec::new()
        },
        // The choices this REQUEST carries and no others: a dry-run has not been
        // asked whether to sort or to write an assertion, so the command it
        // advertises is the plain import it is previewing.
        cli_command: cli_invocation(&CliRun {
            input: plan.staged.upload_name(),
            plan: request,
            root_journal: plan
                .root_journal_id()
                .filter(|id| id != &request.journal_id),
            write_assertion: false,
            sort: false,
            dry_run: false,
            no_git: false,
        }),
        // `applied: false` throughout: a dry-run previews and writes nothing,
        // the same rule every other field on this screen follows.
        id_matches: ids.as_ref().map(|ids| ids.wire(&plan.redactor, false)),
    })))
}

/// The same import proposed **without** hledger's date-based dedup: the run in a
/// directory with no `.latest` beside the CSV.
///
/// Two callers want it and it is one subprocess, so it is run once and shared.
/// [`skipped_by_dedup`] wants the *count* — the difference between the two
/// proposals is exactly what dedup removed, measured rather than inferred, which
/// is the only way to be sure because hledger reports a dropped row nowhere at
/// all. [`reconcile_ids`] wants the *entries*, and for a sharper reason: the rows
/// `.latest` hides are precisely the ones an id match has something to say about.
/// A hold that settled two weeks ago is behind the marker, so the ordinary
/// proposal does not contain it and no amount of matching against that proposal
/// could ever notice. That is `TODO.md`'s bug, restated as a data-flow fact.
///
/// `None` when the destination carries no dedup state: there is then nothing
/// `.latest` could be hiding, and the ordinary proposal already *is* this one.
/// Also `None` when hledger refused the second run, which is the same
/// "say nothing rather than guess" answer this function has always given.
fn bare_proposal(plan: &Plan) -> Result<Option<BareProposal>, AppError> {
    let Some(marker) = stage::latest_marker(plan.destination_dir(), &plan.csv_name) else {
        return Ok(None);
    };
    let bare = plan
        .staged
        .materialize(RUN_BARE, &plan.csv_name, None, plan.skip)
        .map_err(stage_failed)?;
    let output = run(
        import_invocation(
            &plan.hledger,
            &plan.target,
            &plan.rules,
            &bare,
            &plan.aliases.merged,
            ImportRun::Preview,
        ),
        "measure what import de-duplication would skip",
    )?;
    if !output.success() {
        return Ok(None);
    }
    Ok(Some(BareProposal {
        marker,
        entries: output.stdout_lossy(),
        status: output.stderr_lossy(),
    }))
}

/// What every row of this statement would import into, ignoring `.latest`.
struct BareProposal {
    /// The newest date `.latest` records — what rows are being hidden *from*.
    marker: String,
    /// hledger's stdout: every row the rules file proposes, deduped by nothing.
    entries: String,
    /// hledger's stderr, for the count fallback [`reported_count`] reads.
    status: String,
}

/// What matching this statement's rows against the journal by id turned up, and
/// the proposal with the rows the journal already holds taken out of it.
struct IdReconciliation {
    /// `entries`, minus every row whose id the journal already carries. When
    /// nothing was removed this is `entries` byte for byte — see
    /// [`reimport::retain_new`].
    entries: String,
    /// How many transactions that leaves: the `count` a preview reports and the
    /// `imported` a commit reports.
    count: usize,
    /// Rows whose only difference is the clearing status, in proposal order.
    flips: Vec<StatusFlip>,
    /// Rows no transaction in the journal claims.
    new: usize,
    /// Rows the journal already holds, identically.
    unchanged: usize,
    /// Rows the journal holds differently, in proposal order: the row's id and
    /// every disagreement, both **raw**. Kept whole and untreated —
    /// [`IdReconciliation::wire`] is the one place that redacts and clips for a
    /// response body, so the count beside the list is always the true one and
    /// there is a single place to check the hygiene was applied.
    conflicting: Vec<(String, Vec<reimport::FieldDiff>)>,
}

/// One clearing status this statement moved.
struct StatusFlip {
    /// The row id, as the rules file wrote it.
    id: String,
    /// What the journal says today.
    from: &'static str,
    /// What this statement says.
    to: &'static str,
    /// The status to write.
    new_status: Status,
    /// Whether the transaction lives in the file this import writes to, which is
    /// the only file a flip is ever applied in. See
    /// [`WireStatusChange::applied`].
    in_target: bool,
}

impl IdReconciliation {
    /// The report for one call site.
    ///
    /// `applied` is `false` for a dry-run, which previews and writes nothing; a
    /// commit passes `true` and each row is then reported as written exactly
    /// when it was in range to be.
    ///
    /// Every string that leaves here goes through [`reported`] first. These are
    /// a bank's own ids and a journal's own descriptions and amounts rather than
    /// paths this server resolved, so security layer 5 is not obviously in play
    /// — which is the argument for applying it here anyway, rather than for
    /// reasoning once that it need not be.
    fn wire(&self, redactor: &Redactor, applied: bool) -> WireIdMatches {
        WireIdMatches {
            new: self.new,
            unchanged: self.unchanged,
            status_changed: self
                .flips
                .iter()
                .take(MAX_ID_REPORTS)
                .map(|flip| WireStatusChange {
                    id: reported(redactor, &flip.id),
                    from: flip.from,
                    to: flip.to,
                    applied: applied && flip.in_target,
                })
                .collect(),
            status_changed_total: self.flips.len(),
            conflicting: self
                .conflicting
                .iter()
                .take(MAX_ID_REPORTS)
                .map(|(id, diffs)| WireConflict {
                    id: reported(redactor, id),
                    diffs: diffs
                        .iter()
                        .take(MAX_ID_DIFFS)
                        .map(|diff| WireFieldDiff {
                            // Our own phrase, not the user's own text.
                            field: diff.field.clone(),
                            existing: reported(redactor, &diff.existing),
                            incoming: reported(redactor, &diff.incoming),
                        })
                        .collect(),
                })
                .collect(),
            conflicting_total: self.conflicting.len(),
        }
    }
}

/// Match this statement's rows against the journal by the id its rules file
/// writes — or answer `None`, which is "behave exactly as before".
///
/// # The opt-in, and where it actually lives
///
/// `None` comes back whenever no proposed row carries the `id` tag, which is
/// every rules file written before this feature existed. Everything downstream
/// then reads the untouched `entries`, so an import with no id in its rules file
/// is byte-for-byte the import it was — not "equivalent to", the same bytes,
/// because [`reimport::retain_new`] hands back the very `&str` it was given when
/// there is nothing to drop.
///
/// It is decided **observationally**, from hledger's own output, rather than by
/// reading the rules file for a `comment id:` line. A rules file that declares
/// one over a column that turns out to be empty then gets the same silence as
/// one that declares nothing, which is the honest answer: there are no ids here.
///
/// # Which proposal is classified, and which is filtered
///
/// They are not the same one, and that is the whole fix. Rows are **classified**
/// against [`bare_proposal`] — the dedup-free run — because the rows worth
/// talking about are exactly the ones `.latest` hides: a hold that settled last
/// week is behind the marker, so it is not in the ordinary proposal at all and
/// matching against that proposal could never see it. Rows are **filtered** out
/// of the ordinary proposal, because that is the text a commit appends.
///
/// The asymmetry is deliberate and is the safety property. Reading the id as
/// authority to *import* a row `.latest` declined would resurrect rows a journal
/// holds untagged — every transaction imported before the rules file grew its
/// `comment id:` line — and duplicate them. So an id may only ever subtract.
fn reconcile_ids(
    state: &AppState,
    plan: &Plan,
    entries: &str,
    bare: Option<&BareProposal>,
) -> Option<IdReconciliation> {
    let proposed = ledgeline_core::parse_journal(entries, "proposed").ok()?;
    // Parsed only when it is a DIFFERENT text: with no `.latest` beside the CSV
    // the two runs produce the same proposal and there is nothing to re-read.
    let dedup_free = match bare {
        Some(bare) => Some(ledgeline_core::parse_journal(&bare.entries, "proposed").ok()?),
        None => None,
    };
    let classified = dedup_free
        .as_ref()
        .map_or(&proposed.transactions, |journal| &journal.transactions);

    let snapshot = state.snapshot();
    let index = reimport::build_index(&snapshot.journal, reimport::ID_TAG);
    let rows = reimport::reconcile(&index, classified, reimport::ID_TAG)?;

    let mut new = 0;
    let mut unchanged = 0;
    let mut flips = Vec::new();
    let mut conflicting = Vec::new();
    for row in &rows {
        match &row.classification {
            RowClassification::New => new += 1,
            RowClassification::Unchanged => unchanged += 1,
            RowClassification::StatusOnly {
                existing_status,
                new_status,
                ..
            } => flips.push(StatusFlip {
                id: row.id.clone(),
                from: reimport::status_word(*existing_status),
                to: reimport::status_word(*new_status),
                new_status: *new_status,
                // Decided here, from the journal as it stands before anything is
                // written, because the answer cannot change afterwards: the only
                // rows an import appends are `New` ones, whose ids are by
                // definition not in this list.
                in_target: index
                    .get(&row.id)
                    .is_some_and(|(txn, _)| txn.source_file == plan.target),
            }),
            // Raw and whole; `IdReconciliation::wire` does the redacting and the
            // clipping, in one place, for both lists.
            RowClassification::Conflicting { diffs, .. } => {
                conflicting.push((row.id.clone(), diffs.clone()));
            }
        }
    }

    let kept = reimport::retain_new(entries, &proposed.transactions, &index, reimport::ID_TAG);
    // The kept text is hledger's own, minus whole transactions, so it is still a
    // journal — `reimport`'s own round-trip tests pin that — and our parser is
    // the authority on how many it holds.
    let count = count_transactions(&kept).unwrap_or(proposed.transactions.len());
    Some(IdReconciliation {
        entries: kept.into_owned(),
        count,
        flips,
        new,
        unchanged,
        conflicting,
    })
}

/// Sync the clearing statuses an id match found, on a **commit**.
///
/// The one place the import pipeline writes an *existing* transaction, and it
/// does so through [`edit_api::set_statuses`](crate::edit_api::set_statuses) —
/// the same `lock_editor` → `bound` → `set_status` → `save_and_publish`
/// sequence, with the same re-sync on a partial failure, that
/// `PATCH /api/transactions/{index}` has always used. Nothing new writes a
/// journal.
///
/// Confined to [`Plan::target`]. That file is the one whose git state was
/// checked before the commit, and the one the commit's own git commit carries,
/// so a flip cannot land anywhere this request had neither permission for nor a
/// way to undo. A status-only match in some other included file is reported with
/// `applied: false` rather than written.
///
/// Runs **after** the append and the catch-up, so it is the last write and a
/// failure here leaves an import that landed and statuses that did not — a state
/// the next run of the same import repairs by itself, which the message says.
fn apply_status_flips(
    state: &AppState,
    plan: &Plan,
    ids: Option<&IdReconciliation>,
) -> Result<(), AppError> {
    let wanted: Vec<(String, Status)> = ids
        .into_iter()
        .flat_map(|ids| ids.flips.iter())
        .filter(|flip| flip.in_target)
        .map(|flip| (flip.id.clone(), flip.new_status))
        .collect();
    if wanted.is_empty() {
        return Ok(());
    }

    // The append moved every transaction after the target file's own end, so a
    // `Tindex` taken before it may now name a neighbour. Re-read, then resolve
    // each row against THAT journal by its id — see `set_statuses`, which takes
    // a callback for exactly this reason.
    let pending = wanted.len();
    if let Some(Err(error)) = state.reopen_editor() {
        return Err(AppError::Internal(format!(
            "the import landed, but the journal could not be re-read to sync {pending} cleared \
             status(es): {}. Run the same import again to retry them — an id match means nothing \
             will be imported twice.",
            plan.redactor.apply(&error.to_string())
        )));
    }
    let target = plan.target.clone();
    crate::edit_api::set_statuses(state, &move |journal| {
        let index = reimport::build_index(journal, reimport::ID_TAG);
        wanted
            .iter()
            .filter_map(|(id, status)| {
                let (txn, carriers) = index.get(id)?;
                // Re-asked of the journal actually being written: exactly one
                // transaction to name, in the file this import may write, and
                // not already saying what would be set.
                (carriers == 1 && txn.source_file == target && txn.status != *status)
                    .then_some((txn.index, *status))
            })
            .collect()
    })
    .map(|_| ())
}

/// A value from a bank's statement or a user's own journal, made safe for a
/// response body: any path this server resolved rewritten back into the handle
/// the caller already has (security layer 5), then bounded.
///
/// Verbatim otherwise, on the same terms as [`WireProposal::entries`] and
/// [`WireRename`]: this is the user's own text coming back to them, and
/// paraphrasing it would defeat the point of showing a diff at all.
fn reported(redactor: &Redactor, value: &str) -> String {
    clipped(&redactor.apply(value))
}

/// [`reported`]'s length bound, on its own so it can be tested as itself.
fn clipped(value: &str) -> String {
    let mut clipped: String = value.chars().take(MAX_FIELD_CHARS).collect();
    if clipped.len() < value.len() {
        clipped.push('…');
    }
    clipped
}

/// How many rows `.latest` dedup would silently drop, and from when.
///
/// Pure, over the two proposals [`bare_proposal`] already measured. `None` when
/// the destination has no dedup state, in which case there is nothing that could
/// have been dropped.
fn skipped_by_dedup(count: usize, bare: Option<&BareProposal>) -> Option<WireSkipped> {
    let bare = bare?;
    let without = count_transactions(&bare.entries)
        .or_else(|| reported_count(&bare.status, "would import"))
        .unwrap_or(count);
    without
        .checked_sub(count)
        .filter(|dropped| *dropped > 0)
        .map(|dropped| WireSkipped {
            older_than: bare.marker.clone(),
            count: dropped,
        })
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
///
/// # It also answers the command-line-parity question, and shares a run to do it
///
/// Two baselines are wanted, and the second is nearly free:
///
/// * **no aliases at all** → what the rules file alone produces, which is what
///   `renames` is measured against;
/// * **exactly the aliases a config file supplies** → what a terminal
///   `hledger import` would produce, which is what [`WireCliParity`] is measured
///   against.
///
/// With no config file the two baselines are the same command line, so the second
/// costs nothing. With one they differ and it costs a third `--dry-run`, and only
/// then. When the merged set is *equal* to the config's, the two command lines
/// are identical and parity holds by construction with no run at all.
fn alias_effect(
    plan: &Plan,
    staged: &Path,
    entries: &str,
) -> Result<Option<WireAliasEffect>, AppError> {
    if plan.aliases.merged.is_empty() {
        return Ok(None);
    }
    let propose = |aliases: &[String], what: &str| -> Result<Option<String>, AppError> {
        let output = run(
            import_invocation(
                &plan.hledger,
                &plan.target,
                &plan.rules,
                staged,
                aliases,
                ImportRun::Preview,
            ),
            what,
        )?;
        Ok(output.success().then(|| output.stdout_lossy()))
    };

    let bare = propose(&[], "measure what the journal's aliases rewrite")?;
    let renames = bare
        .as_deref()
        .map(|bare| renames_between(bare, entries))
        .unwrap_or_default();

    // What a plain command-line `hledger import` would propose.
    let cli = if plan.aliases.identical_to_conf() {
        None
    } else if plan.aliases.conf.is_empty() {
        bare
    } else {
        propose(
            &plan.aliases.conf,
            "measure what a command-line import would produce",
        )?
    };
    let differences = cli
        .as_deref()
        .map(|cli| renames_between(cli, entries))
        .unwrap_or_default();

    let rename = |(from, to): (String, String)| WireRename {
        from: plan.redactor.apply(&from),
        to: plan.redactor.apply(&to),
    };
    Ok(Some(WireAliasEffect {
        forwarded: plan.aliases.merged.len(),
        renames: renames.into_iter().map(rename).collect(),
        cli: cli_parity(plan, differences.into_iter().map(rename).collect()),
    }))
}

/// The divergence notice, and the fix it offers.
///
/// `differences` is the measurement and is the only thing that decides whether
/// there is anything to say. Nothing here compares alias strings to reach that
/// verdict — a user who wrote an equivalent mapping into their config in a
/// spelling of their own gets silence, which is the correct response to a config
/// that already works.
///
/// The additions are computed only when there IS a divergence, and each is
/// [`hledger_conf::conf_argument`]'s output for one of the journal's aliases: the
/// same bytes the write route will produce, so what the screen shows is what the
/// file gets. An alias that cannot be expressed in a config file at all is listed
/// with its reason rather than dropped.
fn cli_parity(plan: &Plan, differences: Vec<WireRename>) -> WireCliParity {
    let matches = differences.is_empty();
    let target = resolve_conf(&plan.root_dir);
    let outside = plan.conf.as_ref().is_some_and(|conf| conf.outside);
    let (additions, refusals) = if matches {
        (Vec::new(), Vec::new())
    } else {
        conf_additions(&plan.declared, &plan.aliases.conf)
    };
    WireCliParity {
        matches,
        differences,
        conf_path: plan.conf.as_ref().map(|conf| conf.id.clone()),
        conf_outside: outside,
        conf_hijacked_by: plan.conf.as_ref().and_then(|conf| conf.hijacked_by.clone()),
        additions,
        refusals,
        revision: target.revision,
        // A config in force ABOVE the journal's directory is reported, never
        // written — but creating one beside the journal is still allowed, and it
        // is what fixes the import, because hledger uses the NEAREST file. The
        // UI says that shadowing will happen; it is not something to do silently.
        writable: target.writable,
    }
}

/// The `--alias` lines a config file is missing, and the aliases that cannot
/// become one.
///
/// "Missing" is compared in the **config file's** form, not the command line's:
/// a plain `PW Roth IRA - 3077=X` is written as `/^PW.Roth.IRA.-.3077($|:)/=X\1`,
/// so comparing the two spellings would offer to add a line that is already
/// there. Compared this way the operation is idempotent — press the button twice
/// and the second press adds nothing — which is the property that matters.
fn conf_additions(
    declared: &[aliases::Forwarded],
    present: &[String],
) -> (Vec<String>, Vec<WireConfRefusal>) {
    let mut additions: Vec<String> = Vec::new();
    let mut refusals: Vec<WireConfRefusal> = Vec::new();
    for alias in declared {
        // Only aliases that reach an import at all. One refused by
        // `ledgeline_core::aliases` (an `end aliases` closed it, it is empty)
        // already has its own reason on the Account Aliases screen; repeating it
        // here as a config problem would name the wrong cause.
        if alias.argument().is_none() {
            continue;
        }
        match hledger_conf::conf_argument(&alias.pattern, &alias.replacement, alias.regex) {
            Ok(argument) if present.contains(&argument) || additions.contains(&argument) => {}
            Ok(argument) => additions.push(argument),
            Err(refusal) => refusals.push(WireConfRefusal {
                pattern: alias.pattern.clone(),
                replacement: alias.replacement.clone(),
                reason: refusal.code(),
                message: refusal.message(),
            }),
        }
    }
    (additions, refusals)
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

/// Refuse an import whose destination carries dedup state this process cannot
/// read.
///
/// **"No `.latest` at all" is not this case**, and telling the two apart is the
/// whole point. A first import of a file of this name has no dedup state,
/// nothing could be skipped, and it proceeds in silence — that is
/// [`stage::Latest::Absent`]. What is refused is a `.latest.NAME` that *exists*
/// and cannot be carried into the run directory, because the alternative is the
/// worst shape a bug in this module takes:
///
/// * the dry-run runs with no dedup state, so every row looks new;
/// * [`skipped_by_dedup`] has no marker either, so the "N rows would be skipped"
///   warning also says nothing;
/// * the commit reads the **real** directory, where the state file still is, and
///   imports what dedup leaves — far fewer rows than the screen promised.
///
/// hledger exits 0 at every step, so nothing else in the pipeline notices. The
/// preview stops being the bytes and there is no error to read.
///
/// A `409` rather than a `500`: nothing here is broken, there is a file on the
/// user's disk in a state this import will not proceed against, and the sentence
/// names it.
fn refuse_unusable_dedup_state(
    destination: &Path,
    csv_name: &str,
    handle: &str,
) -> Result<(), AppError> {
    let dir = destination.parent().unwrap_or_else(|| Path::new("."));
    match stage::latest_state(dir, csv_name) {
        stage::Latest::Absent | stage::Latest::Present(_) => Ok(()),
        stage::Latest::Unusable(reason) => Err(AppError::Conflict(format!(
            "{} records which rows of {} hledger has already imported, and it {reason}. Going \
             ahead without it would treat every row as new, so nothing was done. Inspect that \
             file, or move it aside to start this statement's de-duplication over.",
            quoted(&stage::latest_name(csv_name)),
            quoted(handle),
        ))),
    }
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
    let Json(body) = compute(move || run_commit(&state, &request, GitPolicy::FromPrefs)).await?;
    Ok(no_store(body))
}

/// The whole of `commit`, synchronously. Every `?` above the CSV write is a
/// decision not to write anything at all.
///
/// # Ledgeline appends; hledger only ever proposes and remembers
///
/// The obvious shape — hand the CSV to `hledger import` and let it append — is
/// not what happens. The write is three steps instead of one:
///
/// 1. `import --dry-run` → the deduped proposal, on stdout, which is **exactly
///    what the dry-run route returned** to the screen;
/// 2. append it with [`edit::atomic_write`](ledgeline_core::edit::atomic_write),
///    byte-compatibly with hledger's own append ([`appended_text`]);
/// 3. `import --catchup` → hledger records `.latest.NAME` itself, so the dedup
///    state stays the thing hledger maintains rather than something Ledgeline
///    now has an opinion about.
///
/// What that buys: **the preview is the bytes**. There is no second rendering
/// that could drift from the one the user approved — and, deliberately, no
/// re-styling step either. Imported amounts keep hledger's own spelling, even
/// when the tree declares a `commodity` style they do not match; `docs/imports.md`
/// § "Commodity style" records why re-printing them is a hazard rather than a
/// polish.
///
/// # What happens if the catch-up fails
///
/// It is the one genuinely new failure this shape introduces. The entries would
/// be in the journal while `.latest` still pointed at the previous import — and
/// the next import of the same statement would propose them **again** and
/// silently duplicate them.
///
/// So a failed catch-up **rolls the journal back** to the bytes read a moment
/// earlier under the write mutex, and reports. The commit becomes all-or-nothing
/// for that failure, which is the property [`preflight_assertion`] already
/// established for a mistyped balance. If the roll-back itself fails, the error
/// says so in as many words and names the duplication risk, because that is a
/// state a person has to be told about rather than one to paper over.
fn run_commit(
    state: &AppState,
    request: &WireCommitRequest,
    git: GitPolicy,
) -> Result<WireCommit, AppError> {
    let plan = Plan::resolve(state, &request.plan)?;
    let targets = plan.targets(&request.plan);
    let prefs = prefs::load();

    // Sequencing rule 3: re-checked HERE. The dry-run's answer is a report, not
    // an authorization, and the UI is not a security boundary.
    let blocked = if git.enabled(&prefs) {
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
    //
    // ALIGNED, and this is the one place where that is not merely a copy's
    // property. The two invocations below read `plan.destination` — they have
    // to, or hledger would key `.latest` to a name in a temp directory — so an
    // unpadded file here is a commit that imports the wrong rows — or none at
    // all — under a genuine `skip 3`, exits 0, and reports the number it got as
    // though it were the number there was. It is also the file the user keeps
    // and re-imports, from this screen or from a terminal, with that same rules
    // file: padding it is what makes those two agree.
    // `convert::align_to_skip` argues the frame mismatch in full.
    let csv = std::fs::read_to_string(plan.staged.data()).map_err(stage_failed)?;
    let csv = convert::align_to_skip(&csv, plan.skip);
    ledgeline_core::edit::atomic_write(&plan.destination, csv.as_bytes()).map_err(|error| {
        AppError::Internal(format!(
            "{} could not be written: {}. Nothing else was changed.",
            quoted(&request.plan.csv_path),
            error.kind()
        ))
    })?;

    let before = std::fs::read(&plan.target).map_err(journal_unreadable)?;
    let output = run(
        import_invocation(
            &plan.hledger,
            &plan.target,
            &plan.rules,
            &plan.destination,
            &plan.aliases.merged,
            ImportRun::Preview,
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
    // The same one step the dry-run route ran, from the same `Plan` — which is
    // what makes the preview the bytes.
    let proposal = output.stdout_lossy();

    // …and the same id reconciliation the dry-run ran, for the same reason: the
    // two must not be able to disagree about which rows are new. It is run here
    // rather than trusted from the preview's response because the UI is not a
    // security boundary — sequencing rule 3's argument, applied to a second
    // thing a client could otherwise skip.
    let bare = bare_proposal(&plan)?;
    let ids = reconcile_ids(state, &plan, &proposal, bare.as_ref());
    let (entries, imported) = match &ids {
        Some(ids) => (ids.entries.as_str(), ids.count),
        None => (
            proposal.as_str(),
            count_transactions(&proposal)
                .or_else(|| reported_count(&output.stderr_lossy(), "would import"))
                .unwrap_or(0),
        ),
    };

    let appended = appended_text(entries);
    if !appended.is_empty() {
        let combined = [before.as_slice(), appended.as_bytes()].concat();
        ledgeline_core::edit::atomic_write(&plan.target, &combined).map_err(|error| {
            AppError::Internal(format!(
                "{} could not be written: {}. The CSV was saved and the journal is unchanged.",
                quoted(&request.plan.journal_id),
                error.kind()
            ))
        })?;
    }

    // hledger's own dedup state, recorded by hledger. Verified byte-identical to
    // what a writing import leaves behind, repeated same-date lines included.
    let complaint = match run(
        import_invocation(
            &plan.hledger,
            &plan.target,
            &plan.rules,
            &plan.destination,
            &plan.aliases.merged,
            ImportRun::Catchup,
        ),
        "record which rows have been imported",
    ) {
        Ok(output) if output.success() => None,
        Ok(output) => Some(plan.redactor.apply(&output.stderr_lossy())),
        Err(error) => Some(error.to_string()),
    };
    if let Some(complaint) = complaint {
        return Err(catchup_failed(&plan, &request.plan, &before, &complaint));
    }

    if request.write_assertion {
        write_assertion(&plan, &request.plan)?;
    }

    // The status sync, and it is deliberately the LAST write: the rows it
    // touches are ones this import did not import, so nothing above depends on
    // it, and a failure here leaves an import that landed rather than a journal
    // half-written. Both reads below then see the flipped bytes — the ordering
    // check because it reads the file, and the git commit because the target is
    // already in its path set. See `apply_status_flips`.
    apply_status_flips(state, &plan, ids.as_ref())?;

    // The ordering check reads the TARGET, and correctly so: date order is a
    // per-file property (`hledger check ordereddates` is per-file too, and a
    // sort that moved a transaction between files would be a different feature
    // entirely). It also cannot fail for an assertion reason, because it never
    // runs hledger — `sort::plan` is our own pure pass over the file's text, so
    // there is no third journal-reading invocation hiding here.
    let text = std::fs::read_to_string(&plan.target).map_err(journal_unreadable)?;
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

    // The dedup marker hledger wrote a moment ago belongs in the SAME commit as
    // the journal and the CSV. `git revert` of an import has to put all three
    // back together: a revert that restores the journal but leaves the marker
    // ahead of it makes the next import of that statement propose nothing, and
    // one that leaves the marker behind makes it propose rows twice.
    //
    // The COMMIT set only — never `blocked_by_git` above. A marker left dirty by
    // an earlier import, or by somebody running `hledger import` in a terminal,
    // is not a reason to refuse every future import through this screen; it is
    // hledger's own bookkeeping, not a user's work that a revert would have to
    // recover. That is why this is a second set rather than a wider `targets`.
    //
    // A marker that does not exist yet (the first import of this name) and one
    // the repository ignores are both non-events: the first is filtered here,
    // the second by `Repo::commit`, which drops ignored paths before staging.
    let marker = plan.marker_target(&request.plan.csv_path);
    let committed: Vec<(&Path, &str)> = targets
        .iter()
        .copied()
        .chain(
            marker
                .iter()
                .map(|(path, handle)| (path.as_path(), handle.as_str())),
        )
        .collect();

    let git = git
        .enabled(&prefs)
        .then(|| {
            commit_targets(
                &committed,
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
        // `applied: true` for every flip that was in range — `apply_status_flips`
        // above returned `Ok`, so those landed, and the ones out of range are
        // reported unwritten.
        id_matches: ids.as_ref().map(|ids| ids.wire(&plan.redactor, true)),
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

/// The error a failed `hledger import --catchup` produces — **after putting the
/// journal back**.
///
/// This is the one failure the append-it-ourselves shape introduces, and it is
/// the dangerous kind: the entries would be in the file while hledger's dedup
/// marker still pointed at the previous import, so the very next import of that
/// statement would propose the same rows again and nobody would question three
/// extra transactions dated last month.
///
/// So the journal is restored to `before` — the bytes read moments earlier,
/// under the write mutex that `commit` and `save-csv` share, so nothing else can
/// have touched the file in between — and the commit reports as a whole. The CSV
/// stays where it was written, which is the same thing a failed import has
/// always left behind.
///
/// A roll-back that itself fails says so in as many words. Reporting loudly is
/// the only honest option there; carrying on and returning `200` would leave a
/// duplication waiting to happen with nothing on screen about it.
fn catchup_failed(
    plan: &Plan,
    request: &WireDryRunRequest,
    before: &[u8],
    complaint: &str,
) -> AppError {
    match ledgeline_core::edit::atomic_write(&plan.target, before) {
        Ok(()) => AppError::Internal(format!(
            "the import was undone because hledger could not record which rows it had already \
             taken. {} is exactly as it was; {} was saved. hledger said:\n{complaint}",
            quoted(&request.journal_id),
            quoted(&request.csv_path),
        )),
        Err(error) => AppError::Internal(format!(
            "hledger could not record which rows it had already taken, and {} could not be put \
             back ({}). The entries ARE in the journal and the de-duplication marker was NOT \
             updated, so importing {} again would add them a second time — check the end of the \
             journal before you do. hledger said:\n{complaint}",
            quoted(&request.journal_id),
            error.kind(),
            quoted(&request.csv_path),
        )),
    }
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
///
/// # The one place the TARGET is read on purpose
///
/// The date comes from `plan.target`, not from the root, and that is not the
/// oversight the rest of this module was. The assertion is appended to the
/// target, so it is dated after the target's own last entry; and the root of a
/// split layout holds no transactions at all, so asking it for a newest date
/// would refuse every assertion in exactly the layouts this fix is about. The
/// *commodity* and the check still come from the root, via [`verify_balance`]
/// and [`check_assertion`], because those are balance questions and the date is
/// not.
fn plan_assertion(
    plan: &Plan,
    request: &WireDryRunRequest,
    proposed: &str,
) -> Result<Option<String>, AppError> {
    let Some((statement, account)) = assertion_fields(request)? else {
        return Ok(None);
    };
    let text = std::fs::read_to_string(&plan.target).map_err(journal_unreadable)?;
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
    let computed = verify_balance(&plan.hledger, &plan.root_journal, proposed, &account)?
        .as_deref()
        .and_then(split_amount);
    assertion_lines(&date, &account, &typed, computed.as_ref()).map(Some)
}

/// Put `assertion` to `hledger check` as ONE journal — the **root**, the entries
/// not yet in it, and the assertion — by the same one-`-f` mechanism
/// [`verify_balance`] uses, never two `-f` flags (fact 3).
///
/// The root for the same reason `verify_balance` uses it: an assertion is a
/// claim about a balance, and a balance is a property of the tree. Checking it
/// against the target alone refused correct statement balances outright in any
/// split layout — the fragment's own start-of-year assertion fails there first,
/// and the user is told their number is wrong when hledger never got as far as
/// looking at it.
///
/// No `--ignore-assertions` here, obviously: evaluating an assertion is the
/// entire job. That it can be evaluated *truthfully* is what reading the root
/// buys.
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
            .stdin(
                concatenated(&plan.root_journal, &format!("{proposed}\n{assertion}")).into_bytes(),
            )
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
            plan.skip,
        )
        .map_err(stage_failed)?;
    let output = run(
        import_invocation(
            &plan.hledger,
            &plan.target,
            &plan.rules,
            &staged,
            &plan.aliases.merged,
            ImportRun::Preview,
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

    let text = std::fs::read_to_string(&plan.target).map_err(journal_unreadable)?;
    let separator = if text.ends_with('\n') { "\n" } else { "\n\n" };
    ledgeline_core::edit::atomic_write(
        &plan.target,
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
    let Json(body) = compute(move || run_sort(&state, &request, GitPolicy::FromPrefs)).await?;
    Ok(no_store(body))
}

/// The whole of `sort`, synchronously.
///
/// # It commits, and it commits SEPARATELY
///
/// A re-sort rewrites the whole journal in place; leaving that uncommitted next
/// to an import that *was* committed means `git diff` no longer shows the import
/// and `git revert` no longer undoes it — the safety net the commit route exists
/// to provide, dismantled by the button offered immediately after it.
///
/// Its own commit rather than an amendment to the import's, because the two are
/// separately undoable and a user who wants the ordering back does not want the
/// transactions back out. Only the journal this call rewrote is in it; the CSV
/// and the dedup marker were not touched here and belong to the import's commit.
///
/// There is deliberately **no `blocked_by_git` pre-flight**. A commit refuses a
/// dirty target because overwriting somebody's uncommitted edit is what a revert
/// could not undo; here the file was just written by the import a moment ago and
/// the user has explicitly asked for this rewrite of it. Blocking would refuse
/// the sort precisely when the import that dirtied the file could not be
/// committed, which is the case where it helps least.
fn run_sort(
    state: &AppState,
    request: &WireSortRequest,
    git: GitPolicy,
) -> Result<WireSorted, AppError> {
    let main = main_journal(state, "journal", &request.journal_id)?;
    let root = include_root(&main)?;
    let (journal, _) = resolve_journal(state, &root, &request.journal_id)?;

    let text = std::fs::read_to_string(&journal).map_err(journal_unreadable)?;
    let plan = sort::plan(&text).map_err(|error| AppError::BadRequest(error.to_string()))?;
    if plan.unchanged {
        return Ok(WireSorted {
            moved: 0,
            git: None,
        });
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

    let redactor = Redactor::default()
        .hide(&journal, &request.journal_id)
        .hide_prefix(&root)
        .hide_prefix(&std::env::temp_dir());
    let targets = vec![(journal.as_path(), request.journal_id.as_str())];
    let git = git
        .enabled(&prefs::load())
        .then(|| {
            commit_targets(
                &targets,
                &sort_message(&request.journal_id, plan.moves.len()),
                &redactor,
            )
        })
        .filter(|result| result.committed || result.message.is_some());

    Ok(WireSorted {
        moved: plan.moves.len(),
        git,
    })
}

/// The generated commit message for a confirmed re-sort.
fn sort_message(journal_id: &str, moved: usize) -> String {
    let name = journal_id.rsplit('/').next().unwrap_or(journal_id);
    let plural = if moved == 1 { "" } else { "s" };
    format!("sort {name} into date order, moving {moved} transaction{plural}")
}

// ===========================================================================
// The command line: one builder, two ends
// ===========================================================================
//
// `ledgeline import` runs an import without a browser, and the dry-run panel
// shows the invocation that would reproduce what is on screen. Those are the two
// ends of one thing, and the failure mode worth designing against is that they
// DRIFT — a displayed command that quietly does something else is worse than no
// command at all, because it is copied into a script and trusted.
//
// So there is exactly one function that knows which flag carries which handle
// ([`cli_argv`]), and exactly one definition of what those flags are
// ([`CliImport`], a `clap` derive). The renderer emits an argv; `clap` parses an
// argv; `a_rendered_command_round_trips_through_clap` runs the first into the
// second. Neither side hand-writes the other's list.

/// One `ledgeline import` run, as the set of choices that define it.
///
/// The handles are the **relative** ones the request already carries — the same
/// strings `Plan::resolve` resolved — never absolute paths. That is the
/// no-path-disclosure rule (§ Security layer 5) applied to a new string on the
/// wire, and it is also what makes the rendered command runnable: the CLI
/// resolves its paths against the process's working directory, so the command
/// reproduces this run when it is run from the journal's own directory, which is
/// what the panel tells the user.
struct CliRun<'a> {
    /// The statement file, by the name it arrived under — see
    /// [`Stage::upload_name`]. It is the one thing here that names a file
    /// outside the journal's tree, and the only honest answer available: a
    /// dropped upload has a name, not a location.
    input: &'a str,
    /// The four handles plus the balance, exactly as resolved.
    plan: &'a WireDryRunRequest,
    /// The root journal's handle, and `None` when it IS the file being written
    /// to. Omitted in that case because `--root-journal 2026.journal -j
    /// 2026.journal` is noise, and because the flag's default is precisely that.
    root_journal: Option<&'a str>,
    write_assertion: bool,
    sort: bool,
    dry_run: bool,
    no_git: bool,
}

/// The argument vector that reproduces `run`, `ledgeline` first.
///
/// **The single source of truth for the flag mapping.** It is what
/// [`cli_invocation`] renders for the screen and what the round-trip test feeds
/// back through `clap`; nothing else anywhere builds an import command line.
///
/// Unquoted, deliberately: this is an argv, the shape `Command::args` takes,
/// where a quote would become part of the file name. Quoting belongs to the
/// display string alone — see [`shell_quote`].
fn cli_argv(run: &CliRun<'_>) -> Vec<String> {
    let mut argv: Vec<String> = ["ledgeline", "import"]
        .into_iter()
        .map(str::to_string)
        .collect();
    let mut push = |flag: &str, value: &str| {
        argv.push(flag.to_string());
        argv.push(value.to_string());
    };
    push("-i", run.input);
    push("-o", &run.plan.csv_path);
    push("-r", &run.plan.rules_id);
    push("-j", &run.plan.journal_id);
    if let Some(root) = run.root_journal {
        push("--root-journal", root);
    }
    if let Some(balance) = run.plan.balance.as_deref() {
        push("--balance", balance);
    }
    if let Some(account) = run.plan.balance_account.as_deref() {
        push("--balance-account", account);
    }
    for (chosen, flag) in [
        (run.write_assertion, "--write-assertion"),
        (run.sort, "--sort"),
        (run.dry_run, "--dry-run"),
        (run.no_git, "--no-git"),
    ] {
        if chosen {
            argv.push(flag.to_string());
        }
    }
    argv
}

/// [`cli_argv`] as one line a person can copy into a shell.
fn cli_invocation(run: &CliRun<'_>) -> String {
    cli_argv(run)
        .iter()
        .map(|argument| shell_quote(argument))
        .collect::<Vec<_>>()
        .join(" ")
}

/// One argument, safe to paste into a POSIX shell.
///
/// Single quotes rather than backslashes because inside `'…'` a shell
/// reinterprets **nothing at all**, so one rule covers spaces, `;`, `$`, `*` and
/// everything else in one go. The apostrophe is the sole exception — it cannot
/// appear inside its own quoting — and the standard spelling for it is to close
/// the quote, escape a bare `'`, and reopen.
///
/// Left bare when there is nothing to protect, so an ordinary handle reads as
/// itself: a command line decorated with quotes it does not need looks like it
/// is hiding something.
fn shell_quote(argument: &str) -> String {
    /// Characters a shell leaves entirely alone, and which therefore need no
    /// quoting. Conservative on purpose — anything not on this list is quoted,
    /// so a character nobody thought about is safe by default rather than
    /// dangerous by default.
    fn is_bare(c: char) -> bool {
        c.is_ascii_alphanumeric() || "._-/:=+,@".contains(c)
    }
    if !argument.is_empty() && argument.chars().all(is_bare) {
        return argument.to_string();
    }
    format!("'{}'", argument.replace('\'', r"'\''"))
}

// ===========================================================================
// `ledgeline import` — the same import, without a browser
// ===========================================================================

/// The flags `ledgeline import` takes.
///
/// **Also the type the runner is handed**, rather than a parallel struct the
/// binary would have to copy into: one definition means the flags a user can
/// type and the choices a run can make are provably the same list.
///
/// Every path is an ordinary filesystem path, resolved against the process's
/// working directory exactly as any other command-line tool resolves one. They
/// are turned into the journal-relative handles the engine works in by
/// [`run_cli_import`], through the *same* resolution the HTTP routes use — the
/// journal target list and the rules discovery scan — so the CLI can name
/// exactly the files the screen can and no others.
#[derive(clap::Args, Debug, Clone)]
pub struct CliImport {
    /// The statement to import: CSV/TSV, OFX/QFX/QBO, or a spreadsheet.
    #[arg(short = 'i', long)]
    input: PathBuf,

    /// Where to keep the converted CSV. Must be inside the journal's own
    /// directory, and is also the file `hledger` keys its de-duplication state
    /// to, so re-importing the same statement later needs the same `--output`.
    #[arg(short = 'o', long)]
    output: PathBuf,

    /// The hledger CSV rules file to import with.
    #[arg(short = 'r', long)]
    rules: PathBuf,

    /// The journal file to append the imported transactions to.
    #[arg(short = 'j', long)]
    journal: PathBuf,

    /// The journal to reckon balances against — the root that `include`s
    /// `--journal`. Defaults to `--journal` itself, which is right for a
    /// single-file journal and wrong for every split one.
    #[arg(long)]
    root_journal: Option<PathBuf>,

    /// The statement's closing balance, which may be negative. The import is
    /// REFUSED if the journal does not reconcile to it.
    // `allow_hyphen_values` because **a credit-card statement balance is
    // negative**, and without it `--balance -3238.65` is read as the unknown
    // flag `-3` — which would make this option unusable for exactly the accounts
    // people most want to reconcile. The same case `plain_field` permits a
    // leading `-` for. Found by the round-trip test rather than by inspection,
    // which is what that test is for. A `//` comment, not a `///` one: this is
    // an argument to the next maintainer, not to someone reading `--help`.
    #[arg(long, allow_hyphen_values = true, requires = "balance_account")]
    balance: Option<String>,

    /// The account `--balance` is a balance of.
    #[arg(long, requires = "balance")]
    balance_account: Option<String>,

    /// Write `--balance` into the journal as a balance assertion.
    #[arg(long, requires = "balance")]
    write_assertion: bool,

    /// Re-sort the journal into date order afterwards, if the import left it out
    /// of order.
    #[arg(long)]
    sort: bool,

    /// Report what would be imported and write nothing at all.
    #[arg(long)]
    dry_run: bool,

    /// Do not commit to git around this import, whatever the preferences say.
    #[arg(long)]
    no_git: bool,
}

impl CliImport {
    /// The journal to OPEN: the root of the `include` tree this import is
    /// reckoned against, which is `--journal` itself unless `--root-journal`
    /// says otherwise.
    ///
    /// The binary needs this before the runner exists — it is what the
    /// [`AppState`] is built from — so the defaulting rule lives here rather
    /// than being spelled out at the one call site that would then own it.
    #[must_use]
    pub fn root_journal_path(&self) -> &Path {
        self.root_journal.as_deref().unwrap_or(&self.journal)
    }
}

/// What a `ledgeline import` run did.
///
/// Everything the binary needs for stdout and an exit code, and nothing about
/// how it is printed — the rendering is `main.rs`'s, so this crate never decides
/// what a terminal looks like.
#[derive(Debug, Clone)]
pub struct CliImportReport {
    /// The `ledgeline import …` line that reproduces this run, from the same
    /// builder the dry-run panel shows. Echoed so a log of a scripted run says
    /// what it did in a form that can be re-run.
    pub command: String,
    /// hledger's own status line for the preview.
    pub status: String,
    /// Transactions the preview proposed.
    pub count: usize,
    /// The statement-balance reconciliation, when one was asked for.
    pub balance: Option<String>,
    /// `None` on `--dry-run`, where the whole point is that there is nothing to
    /// report because nothing was written.
    pub written: Option<CliImportWritten>,
}

/// What a committing run actually wrote.
#[derive(Debug, Clone)]
pub struct CliImportWritten {
    /// The CSV's handle, relative to the journal's directory.
    pub csv: String,
    /// The journal's handle, likewise.
    pub journal: String,
    /// Transactions appended.
    pub imported: usize,
    /// The journal is in date order after the import.
    pub in_order: bool,
    /// Transactions a `--sort` moved, or `None` when it was not asked for.
    pub sorted: Option<usize>,
}

/// Run one non-interactive import against `state`.
///
/// # Why this reuses the HTTP routes' own functions
///
/// It stages the file through [`stage_upload`] and then calls
/// [`run_dry_run`]/[`run_commit`]/[`run_sort`] — the very functions the axum
/// handlers call, with the very request types they deserialize. Nothing about
/// the import sequence is re-implemented here, and there is no second code path
/// that could import differently: `docs/imports.md` describes one pipeline, and
/// this is a second caller of it rather than a second copy of it. In particular
/// every subprocess still goes through `hledger.rs`/`git.rs`, which stay the only
/// two modules in this crate that may spawn one.
///
/// The only genuinely new work is turning command-line **paths** into the
/// journal-relative **handles** the engine speaks, and it is done by asking the
/// same two scans the routes ask.
///
/// # No write mutex, and why that is not an omission
///
/// The `commit` and `sort` handlers take `AppState::import_writes` because two
/// concurrent HTTP requests can reach them and would interleave hledger's
/// appends. This process has one import in it and no socket, so there is no
/// second writer to serialize against and the guard would only ever be
/// uncontended. Two `ledgeline import` processes racing on one journal are not
/// covered by an in-process mutex in either design.
///
/// # The dry run always happens
///
/// Even for a committing run, which costs one extra `hledger import --dry-run`.
/// It buys two things worth more than the subprocess: the report says what is
/// about to happen in the same words the screen would, and a `--balance` that
/// does not reconcile can refuse **before** anything is written. A script has
/// nobody to look at a red number and decide.
///
/// # Errors
///
/// One sentence, ready to print. The engine's own [`AppError`] is deliberately
/// not exposed: its variants are HTTP conditions, which a command line has no
/// use for, and its `Display` is already the sentence a person needs.
pub fn run_cli_import(state: &AppState, args: &CliImport) -> Result<CliImportReport, String> {
    cli_import(state, args).map_err(|error| error.to_string())
}

/// [`run_cli_import`], in the crate's own error type.
fn cli_import(state: &AppState, args: &CliImport) -> Result<CliImportReport, AppError> {
    // The journal this process was opened with — `--root-journal`, or `--journal`
    // when it was not given. `main_journal`'s own 404 is not used here: its
    // wording is for a caller that supplied a handle, and this state was built
    // from a parsed journal file, so an empty source list is impossible rather
    // than merely unlikely.
    let root_journal = state
        .source_files()
        .into_iter()
        .next()
        .ok_or_else(|| AppError::Internal("this process has no journal open".to_string()))?;
    let root_dir = include_root(&root_journal)?;

    // Layer 2/3 resolution, through the SAME scans the routes use: a path that
    // does not name a file the engine already knows about cannot be imported to,
    // whichever door it arrives at.
    let journal_id = cli_journal_id(state, &root_dir, &args.journal)?;
    let rules_id = cli_rules_id(&rules::discover(&root_journal), &args.rules)?;
    let csv_path = cli_csv_path(&root_dir, &args.output)?;
    let root_id = cli_journal_id(state, &root_dir, &root_journal)?;

    // The upload, done the way the browser does it — the same conversion, the
    // same detection, the same staging area — because a CLI import that read the
    // file some other way would be a different import.
    let name = cli_upload_name(&args.input)?;
    let bytes = std::fs::read(&args.input).map_err(|error| {
        AppError::BadRequest(format!(
            "{} could not be read: {}",
            quoted(&args.input.display().to_string()),
            error.kind()
        ))
    })?;
    if bytes.len() > stage::MAX_UPLOAD_BYTES {
        return Err(AppError::BadRequest(format!(
            "this file is larger than the {} MiB import limit",
            stage::MAX_UPLOAD_BYTES / (1024 * 1024)
        )));
    }
    let staged = stage_upload(state, &name, &bytes)?;

    let request = WireDryRunRequest {
        stage_id: staged.stage_id,
        rules_id,
        csv_path,
        journal_id,
        balance: args.balance.clone(),
        balance_account: args.balance_account.clone(),
    };
    let git = if args.no_git {
        GitPolicy::Off
    } else {
        GitPolicy::FromPrefs
    };
    let command = cli_invocation(&CliRun {
        input: &name,
        plan: &request,
        root_journal: (root_id != request.journal_id).then_some(root_id.as_str()),
        write_assertion: args.write_assertion,
        sort: args.sort,
        dry_run: args.dry_run,
        no_git: args.no_git,
    });

    let (count, status, balance) = match run_dry_run(state, &request, git)? {
        WireDryRun::Failed(failed) => {
            return Err(AppError::BadRequest(format!(
                "the import preview failed and nothing was written. hledger said:\n{}",
                failed.stderr
            )));
        }
        WireDryRun::Proposed(proposal) => {
            // A statement balance that does not reconcile REFUSES the run, which
            // is stricter than the screen — there the number is shown in red and
            // the person decides. A script has no such person, and "imported
            // anyway, into books that no longer agree with the statement" is not
            // a thing to do quietly.
            if let Some(balance) = &proposal.balance
                && !balance.matches
            {
                return Err(AppError::BadRequest(format!(
                    "the statement balance {} does not match the journal's {}, so nothing was \
                     written. Difference: {}.",
                    balance.statement,
                    balance.computed,
                    balance.difference.as_deref().unwrap_or("not a number"),
                )));
            }
            let balance = proposal
                .balance
                .as_ref()
                .map(|balance| format!("{} matches the journal", balance.computed));
            (proposal.count, proposal.status, balance)
        }
    };

    if args.dry_run {
        return Ok(CliImportReport {
            command,
            status,
            count,
            balance,
            written: None,
        });
    }

    let journal_id = request.journal_id.clone();
    let commit = run_commit(
        state,
        &WireCommitRequest {
            plan: request,
            write_assertion: args.write_assertion,
        },
        git,
    )?;

    // Only when it is both asked for and needed. `run_sort` is a no-op on a
    // journal already in order, but saying so costs a whole-file read and a
    // second git commit message about nothing.
    let sorted = match (args.sort, commit.ordering.in_order) {
        (true, false) => Some(
            run_sort(
                state,
                &WireSortRequest {
                    journal_id: journal_id.clone(),
                },
                git,
            )?
            .moved,
        ),
        (true, true) => Some(0),
        (false, _) => None,
    };

    Ok(CliImportReport {
        command,
        status,
        count,
        balance,
        written: Some(CliImportWritten {
            csv: commit.csv_written,
            journal: commit.journal_written,
            imported: commit.imported,
            in_order: commit.ordering.in_order || sorted.is_some(),
            sorted,
        }),
    })
}

/// The `--input` file's own name, validated exactly as an upload's
/// `X-Ledgeline-Filename` is.
///
/// The same check rather than a laxer one because the name does the same two
/// jobs here that it does there — it decides the format and it is echoed back —
/// and because a CLI is not a reason to relax a rule the HTTP surface keeps.
fn cli_upload_name(input: &Path) -> Result<String, AppError> {
    let malformed = || {
        AppError::BadRequest(format!(
            "{} does not end in a usable file name",
            quoted(&input.display().to_string())
        ))
    };
    let name = input
        .file_name()
        .ok_or_else(malformed)?
        .to_str()
        .ok_or_else(malformed)?;
    let well_formed = !name.is_empty()
        && name.len() <= MAX_FILENAME_BYTES
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', ':'])
        && !name.chars().any(|c| c.is_ascii_control());
    well_formed.then(|| name.to_string()).ok_or_else(malformed)
}

/// Which journal handle `path` names, by matching it against the same target
/// list [`resolve_journal`] resolves against.
///
/// Deliberately a search of [`journals::targets`] rather than
/// `path.strip_prefix(root)`: the handles are the engine's, derived from the
/// files this parse actually read, so a path that is not one of them is not a
/// file this journal includes — and saying which ones it *does* include is the
/// useful half of that refusal.
fn cli_journal_id(state: &AppState, root_dir: &Path, path: &Path) -> Result<String, AppError> {
    let wanted = std::fs::canonicalize(path).map_err(|error| {
        AppError::BadRequest(format!(
            "{} could not be resolved: {}",
            quoted(&path.display().to_string()),
            error.kind()
        ))
    })?;
    let snapshot = state.snapshot();
    let targets = journals::targets(&snapshot.journal);
    targets
        .iter()
        .find(|target| {
            std::fs::canonicalize(root_dir.join(&target.id)).is_ok_and(|known| known == wanted)
        })
        .map(|target| target.id.clone())
        .ok_or_else(|| {
            AppError::BadRequest(format!(
                "{} is not part of this journal. It includes: {}",
                quoted(&path.display().to_string()),
                targets
                    .iter()
                    .map(|target| target.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })
}

/// Which rules-file handle `path` names, by matching it against the discovery
/// scan — the only id → rules-file resolution this codebase has.
fn cli_rules_id(discovery: &Discovery, path: &Path) -> Result<String, AppError> {
    let wanted = std::fs::canonicalize(path).map_err(|error| {
        AppError::BadRequest(format!(
            "{} could not be resolved: {}",
            quoted(&path.display().to_string()),
            error.kind()
        ))
    })?;
    discovery
        .files
        .iter()
        .find(|found| {
            discovery
                .resolve(&found.id)
                .and_then(|found| std::fs::canonicalize(found.path().as_path()).ok())
                .is_some_and(|known| known == wanted)
        })
        .map(|found| found.id.clone())
        .ok_or_else(|| {
            AppError::BadRequest(format!(
                "{} is not a rules file beside this journal. Found: {}",
                quoted(&path.display().to_string()),
                if discovery.files.is_empty() {
                    "none".to_string()
                } else {
                    discovery
                        .files
                        .iter()
                        .map(|found| found.id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                }
            ))
        })
}

/// The `csvPath` handle for `path`: its location relative to the journal's own
/// directory, with `/` separators.
///
/// The one handle naming a file that need not exist, so — exactly as
/// [`resolve_destination`] does for the same reason — the **parent** is
/// canonicalized and required to be inside the root, and the file name is joined
/// on afterwards. `resolve_destination` then re-checks all of it when the plan
/// resolves; this only has to produce a handle, not to trust one.
fn cli_csv_path(root_dir: &Path, path: &Path) -> Result<String, AppError> {
    let outside = || {
        AppError::BadRequest(format!(
            "{} is not inside this journal's own directory, so an import cannot write there",
            quoted(&path.display().to_string())
        ))
    };
    let (parent, name) = path
        .parent()
        .zip(path.file_name().and_then(|name| name.to_str()))
        .ok_or_else(outside)?;
    let parent = if parent.as_os_str().is_empty() {
        Path::new(".")
    } else {
        parent
    };
    let parent = std::fs::canonicalize(parent).map_err(|_| outside())?;
    let relative = parent.strip_prefix(root_dir).map_err(|_| outside())?;
    let mut components: Vec<String> = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect();
    components.push(name.to_string());
    Ok(components.join("/"))
}

/// `POST /api/import/hledger-conf` — install the journal's aliases into an
/// `hledger.conf` beside it, so a terminal `hledger import` maps the same
/// accounts this screen does.
///
/// A **new write target**, and so it gets the same discipline as every other one
/// in this crate rather than a lighter one because the file is small:
///
/// 1. **Editing must be enabled**, exactly as for a journal or a rules file.
/// 2. **The location is fixed** — `hledger.conf` in the journal's own directory.
///    No component comes from the client, so there is no handle to validate and
///    no path arithmetic to get wrong; `$HOME` and the XDG config dir are never
///    written. See [`resolve_conf`].
/// 3. **Confinement, file type, symlinks** — [`parse::confine`], then
///    `symlink_metadata`: absent or a regular file. See [`resolve_conf`].
/// 4. **Content provenance** — the request carries a revision and nothing else.
///    Every byte written is either a byte read from that file moments ago or
///    [`hledger_conf`]'s own rendering of an `alias` directive the journal
///    already declares.
/// 5. **Revision / 409** — the same model as the rules and alias editors, with
///    the empty string as the revision of a file that does not exist, re-checked
///    immediately before the write.
pub(crate) async fn write_hledger_conf(
    State(state): State<AppState>,
    payload: Result<Json<WireConfRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    let request = json_body(payload)?;
    if !state.editing_enabled() {
        return Err(crate::error::editing_disabled());
    }
    // The same mutex the import and alias writes take. A config file is read by
    // every hledger invocation an import makes, so rewriting one while an import
    // is in flight would change that import's rules underneath it.
    let guard = state.clone();
    let _write = guard.import_writes().lock().await;
    let Json(body) = compute(move || run_write_conf(&state, &request)).await?;
    Ok(no_store(body))
}

/// The whole of the config write, synchronously. Every `?` is a decision to
/// write nothing.
fn run_write_conf(
    state: &AppState,
    request: &WireConfRequest,
) -> Result<WireConfWritten, AppError> {
    let main = main_journal(state, "journal", hledger_conf::CONF_NAME)?;
    let root = include_root(&main)?;
    let target = resolve_conf(&root);
    if !target.writable {
        return Err(AppError::BadRequest(format!(
            "{} cannot be written: a config file must be a regular file in your journal's own \
             directory, not a symlink or a directory",
            quoted(&target.id)
        )));
    }
    if target.revision != request.revision {
        return Err(AppError::Conflict(format!(
            "{} changed on disk since this page read it, so nothing was written. Reload and try \
             again.",
            quoted(&target.id)
        )));
    }

    // Layer 4: recomputed here, never taken from the request.
    let declared = aliases::forward(&state.snapshot().journal);
    let present = hledger_conf::alias_arguments(&target.text, hledger_conf::IMPORT_COMMAND);
    let (additions, _) = conf_additions(&declared, &present);
    if additions.is_empty() {
        // Nothing to add writes NOTHING — not even byte-identical content, which
        // would still bump mtime and wake somebody's watch loop. The same lesson
        // `rules_api` and `alias_api` both record.
        return Ok(WireConfWritten {
            conf_path: target.id,
            created: false,
            added: Vec::new(),
            revision: target.revision,
        });
    }

    let base = if target.exists {
        target.text.clone()
    } else {
        hledger_conf::new_file_header()
    };
    let new_text = hledger_conf::with_aliases(&base, &additions);

    // Narrow the TOCTOU window from "the whole request" to "read → rename", the
    // same last-moment re-check `alias_api` makes.
    let before = resolve_conf(&root);
    if before.revision != request.revision || !before.writable {
        return Err(AppError::Conflict(format!(
            "{} changed on disk since this page read it, so nothing was written. Reload and try \
             again.",
            quoted(&target.id)
        )));
    }

    ledgeline_core::edit::atomic_write(&target.path, new_text.as_bytes()).map_err(|error| {
        // Only the `ErrorKind`: `atomic_write` builds a temp path from the
        // target, so its io errors can carry one.
        AppError::Internal(format!(
            "{} could not be written: {}. Nothing else was changed.",
            quoted(&target.id),
            error.kind()
        ))
    })?;

    Ok(WireConfWritten {
        conf_path: target.id,
        created: !target.exists,
        added: additions,
        // From what we WROTE, never from a re-read: a re-read could pick up
        // somebody else's write and hand this client a token for bytes it has
        // never seen, which is how the next write clobbers that person silently.
        revision: Fingerprint::of_bytes(new_text.as_bytes()).token(),
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

    /// **Rendering money is total: neither the requested scale nor the value's
    /// own can turn a short input into an unbounded allocation.**
    ///
    /// Built with [`Dec::new`] rather than by parsing, deliberately. `Dec::parse`
    /// caps scale at `MAX_PARSE_PLACES`, so an end-to-end test through
    /// [`reconcile`] would pass whether or not this function clamps anything —
    /// it would be testing the parser. The property here is that
    /// [`render_money_at`] is bounded *on its own terms*, because it is a
    /// renderer and renderers do not get to assume their caller validated
    /// something.
    ///
    /// The sibling defect: an unclamped `places` feeding `"0".repeat(…)` in
    /// `convert::ofx` turned a 345-byte statement into a 20 MB render. This one
    /// had two such repeats — one for the requested scale, one for the value's.
    #[test]
    fn rendering_money_is_bounded_whatever_scale_it_is_handed() {
        // The bound, in bytes: a sign, an integer part, a point, and at most
        // `MAX_RENDER_PLACES` fractional digits. `i128` tops out at 39 digits.
        let ceiling = usize::try_from(MAX_RENDER_PLACES).expect("fits") + 64;

        // 1. An absurd scale on the VALUE — the second `repeat`.
        let deep = Dec::new(12345, u32::MAX);
        assert!(
            render_money_at(deep, 2).len() <= ceiling,
            "a value scale of u32::MAX must not allocate proportionally"
        );

        // 2. An absurd REQUESTED scale — the first `repeat`, which pads the
        //    mantissa out to the width asked for.
        let ordinary = Dec::new(294_505, 2);
        assert!(
            render_money_at(ordinary, u32::MAX).len() <= ceiling,
            "a requested scale of u32::MAX must not allocate proportionally"
        );

        // 3. Both at once.
        assert!(render_money_at(deep, u32::MAX).len() <= ceiling);

        // And the clamp is inert for every scale a real balance carries: below
        // the bound the function still refuses to truncate, which is the
        // property the reconciliation depends on.
        assert_eq!(render_money_at(ordinary, 0), "2945.05", "no truncation");
        assert_eq!(render_money_at(ordinary, 4), "2945.0500", "pads instead");
        assert_eq!(render_money_at(Dec::new(-500, 2), 2), "-5.00");
        assert_eq!(render_money_at(Dec::zero(), 2), "0.00");
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

    /// **The bytes Ledgeline appends are the bytes hledger would have.**
    ///
    /// Pinned against a real `hledger import` run: a dry-run's stdout ends in a
    /// blank line, and the append is a leading newline plus that text with the
    /// blank line removed. Getting this wrong would show up as a creeping blank
    /// line at the end of a journal, once per import, forever.
    #[test]
    fn the_appended_bytes_match_hledgers_own_append() {
        // Verbatim from `hledger 1.52 import --dry-run` — note the trailing
        // blank line, which is hledger's, not a typo here.
        let stdout = "2026-02-01 GROCERY STORE\n    assets:bank:checking   $-405\n\
                      \x20   expenses:unknown        $405\n\n";
        assert_eq!(
            appended_text(stdout),
            "\n2026-02-01 GROCERY STORE\n    assets:bank:checking   $-405\n\
             \x20   expenses:unknown        $405\n",
        );

        // Exactly one trailing newline, however many the text arrived with.
        assert_eq!(appended_text("2026-02-01 A\n"), "\n2026-02-01 A\n");
        assert_eq!(appended_text("2026-02-01 A\n\n\n"), "\n2026-02-01 A\n");

        // Nothing proposed appends NOTHING — the same thing hledger does with a
        // statement holding no new rows.
        assert_eq!(appended_text(""), "");
        assert_eq!(appended_text("\n\n  \n"), "");
    }

    /// The count comes from the text that is actually appended, so it can never
    /// disagree with what landed.
    #[test]
    fn the_imported_count_is_the_entries_that_were_appended() {
        let entries = "2026-01-01 A\n    a  $1.00\n    b  $-1.00\n\n\
                       2026-02-01 B\n    a  $2.00\n    b  $-2.00\n\n";
        assert_eq!(count_transactions(entries), Some(2));
        assert_eq!(count_transactions(""), Some(0));
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

    /// **A balance account may not smuggle a `;` into the journal.**
    ///
    /// The account is written verbatim into the posting line `assertion_lines`
    /// appends, and an account name has no end-of-line comment: it runs to the
    /// two-space separator. So `assets:bank ; note` is not an account with a
    /// note attached, it is one account whose NAME contains a semicolon —
    /// verified against hledger 1.52, where `hledger accounts` lists
    /// `assets:bank ; note` beside `assets:bank`.
    ///
    /// `check_assertion` does not save us: a phantom account holds nothing, so a
    /// `$0` statement balance asserts truthfully against it, `hledger check`
    /// exits 0, and the line is committed. This is the defect that already
    /// reached a real journal through the rules-file door.
    #[test]
    fn a_balance_account_may_not_carry_a_comment() {
        let refused = argument_field("assets:bank ; note", "balance account")
            .expect_err("a `;` in an account name must be refused");
        let message = refused.to_string();
        // The message must teach WHY, not just say "invalid character": the
        // point the user has to learn is that it becomes part of the name.
        assert!(message.contains(";"), "{message}");
        assert!(message.contains("NAME"), "{message}");
        assert!(
            matches!(refused, AppError::BadRequest(_)),
            "a bad field is the caller's fault, not a 500"
        );

        // Two spaces in a row end the account name and start the amount.
        assert!(argument_field("assets:bank  checking", "balance account").is_err());

        // A newline would inject a whole journal LINE. `plain_field` already
        // refuses it as a control character; this pins that it stays refused
        // through the account validator too, since that is the one that guards
        // the write.
        assert!(argument_field("assets:bank\n2026-01-01 fake", "balance account").is_err());
        assert!(argument_field("assets:bank\tchecking", "balance account").is_err());

        // ...and the legitimate names all still pass. `#` is NOT refused: it
        // opens a comment only at the start of a line, and the account is always
        // written indented, so `assets:card #1234` round-trips intact through
        // hledger 1.52. Single interior spaces are ordinary in account names.
        for good in [
            "assets:bank:checking",
            "assets:card #1234",
            "expenses:home office",
            "liabilities:credit card:visa",
            "assets:banque:épargne",
        ] {
            assert_eq!(
                argument_field(good, "balance account").as_deref(),
                Ok(good),
                "{good:?} is a real account name and must pass"
            );
        }

        // Surrounding whitespace is trimmed rather than refused, as it is for
        // every other field — a pasted account name usually brings some.
        assert_eq!(
            argument_field("  assets:bank:checking  ", "balance account").as_deref(),
            Ok("assets:bank:checking")
        );
    }

    // -----------------------------------------------------------------------
    // The `--no-conf` lint
    // -----------------------------------------------------------------------

    /// A stand-in `Hledger` for building argv without running anything.
    ///
    /// `resolve` is the only public constructor and it spawns a process, so this
    /// reaches for the private fields directly — which is exactly what a
    /// same-crate test may do and an outside caller may not. Nothing here runs.
    fn argv_of(invocation: &Invocation) -> Vec<String> {
        invocation
            .argv()
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    /// **Every hledger invocation the import path builds passes `--no-conf`.**
    ///
    /// An argument-level lint rather than a behavioural test, deliberately. The
    /// behaviour it stands for is "a `hledger.conf` sitting above our working
    /// directory does not steer our subprocess", and proving *that* needs a
    /// hostile config file planted above the test runner's working directory —
    /// which is this repository, on a developer's machine and in CI. A test that
    /// writes `hledger.conf` into a parent of the checkout to prove a point is a
    /// test that breaks every other test in the run.
    ///
    /// So the property is enforced at the choke point
    /// ([`Invocation::argv`](crate::hledger::Invocation::argv)) and asserted
    /// here over the real builders. A new invocation added to this module fails
    /// this test only if it is also added to the list — which is the honest
    /// limit of an argument-level lint, and why the flag is prepended by
    /// construction rather than by anyone remembering to.
    ///
    /// The manual verification is in `docs/imports.md`: a `hledger.conf` whose
    /// entire content is the word `balance` rewrites an unprotected
    /// `hledger import` into `balance import …`.
    #[test]
    fn every_import_invocation_disables_config_files() {
        let hledger = Hledger::for_tests(Path::new("/nonexistent/hledger"));
        let journal = Path::new("/j/main.journal");
        let rules = Path::new("/j/bank.csv.rules");
        let csv = Path::new("/j/bank.csv");
        let aliases = vec!["a=b".to_string()];

        // One entry per `.invoke(` in this module's production half, mirroring
        // each builder's own shape. `the_lint_covers_every_builder` below is what
        // keeps this list from silently falling behind.
        let built = vec![
            // `import_invocation` — shared by the preview, the dedup
            // measurement, the alias measurements and the catch-up.
            import_invocation(&hledger, journal, rules, csv, &aliases, ImportRun::Preview),
            import_invocation(&hledger, journal, rules, csv, &aliases, ImportRun::Catchup),
            // `print_json` — candidate scoring.
            hledger
                .invoke(alias_flags(&aliases))
                .args(["print".as_ref(), "-f".as_ref(), csv.as_os_str()])
                .arg("--rules")
                .arg(rules)
                .args(["-O", "json"]),
            // `verify_balance` — the journal and the proposal, through a pipe.
            hledger
                .invoke(["-f", "-", "balance"])
                .arg("assets:bank")
                .args(["--no-total", "--flat", "-O", "csv"]),
            // `check_assertion` — the same pipe, one commodity later.
            hledger.invoke(["-f", "-", "check"]),
        ];

        for invocation in &built {
            let argv = argv_of(invocation);
            assert_eq!(
                argv.first().map(String::as_str),
                Some(crate::hledger::NO_CONF),
                "{argv:?} must begin with {} — a config file can otherwise replace the command",
                crate::hledger::NO_CONF
            );
            assert_eq!(
                argv.iter()
                    .filter(|arg| *arg == crate::hledger::NO_CONF)
                    .count(),
                1,
                "{argv:?} should carry the flag exactly once"
            );
        }
    }

    /// The lint above mirrors each builder by hand, so this counts the builders
    /// and fails when a new one appears without a mirror.
    ///
    /// Reading this module's own source is unusual and is the point: the
    /// alternative is a list that is correct on the day it is written and quietly
    /// incomplete a month later, which for a security property is worse than no
    /// list. `Hledger::invoke` is the only way to construct an [`Invocation`]
    /// outside `hledger.rs`, so counting its call sites counts the builders.
    #[test]
    fn the_lint_covers_every_builder() {
        /// How many `.invoke(` call sites `every_import_invocation_disables_config_files`
        /// mirrors. Bump this **and add the builder to that test** together.
        const MIRRORED: usize = 4;

        let source = include_str!("import_api.rs");
        let production = source
            .split_once("\n#[cfg(test)]\n")
            .map_or(source, |(before, _)| before);
        assert_eq!(
            production.matches(".invoke(").count(),
            MIRRORED,
            "a new hledger invocation was added to import_api.rs; mirror it in \
             `every_import_invocation_disables_config_files` so its argv is linted too"
        );
    }

    /// The flag goes in front of the SUBCOMMAND, not merely somewhere in the
    /// vector. A config file's own injected command word is prepended to the
    /// argument list, so a `--no-conf` sitting after `import` would be parsed
    /// only once the damage was already done.
    #[test]
    fn the_flag_precedes_the_subcommand() {
        let hledger = Hledger::for_tests(Path::new("/nonexistent/hledger"));
        let argv = argv_of(&import_invocation(
            &hledger,
            Path::new("/j/main.journal"),
            Path::new("/j/b.csv.rules"),
            Path::new("/j/b.csv"),
            &[],
            ImportRun::Preview,
        ));
        let flag = argv
            .iter()
            .position(|arg| arg == crate::hledger::NO_CONF)
            .expect("the flag");
        let import = argv.iter().position(|arg| arg == "import").expect("import");
        assert!(flag < import, "{argv:?}");
    }

    // -----------------------------------------------------------------------
    // `--ignore-assertions`: on the import, and on nothing else
    // -----------------------------------------------------------------------

    /// The import — preview AND write — disables assertion checking, and the
    /// flag precedes the subcommand because it is a general option.
    ///
    /// The behavioural half of this is
    /// `import_endpoints.rs::an_import_into_a_year_file_succeeds_despite_its_start_of_year_assertion`,
    /// which needs a real hledger. This is the argument-level half, and it is
    /// what a reader tempted to delete the flag will trip over first.
    #[test]
    fn the_import_ignores_assertions_and_the_flag_precedes_the_subcommand() {
        let hledger = Hledger::for_tests(Path::new("/nonexistent/hledger"));
        for run in [ImportRun::Preview, ImportRun::Catchup] {
            let argv = argv_of(&import_invocation(
                &hledger,
                Path::new("/j/2026/2026.journal"),
                Path::new("/j/b.csv.rules"),
                Path::new("/j/b.csv"),
                &[],
                run,
            ));
            let flag = argv
                .iter()
                .position(|arg| arg == IGNORE_ASSERTIONS)
                .unwrap_or_else(|| {
                    panic!(
                        "{argv:?} must carry {IGNORE_ASSERTIONS}: a target file's own assertions \
                         cannot be evaluated when it is read alone. See `import_invocation`."
                    )
                });
            let import = argv.iter().position(|arg| arg == "import").expect("import");
            assert!(flag < import, "{argv:?}: it is a general option");
        }
    }

    /// **Nothing that reads a journal for a balance disables assertions.**
    ///
    /// The complement of the test above, and the one that keeps the flag from
    /// spreading. `verify_balance` and `check_assertion` read the ROOT, where an
    /// assertion is in the context it was written for and is therefore
    /// meaningful — a failure there is real information, and silencing it would
    /// turn "this journal does not hold together" into a confident number.
    #[test]
    fn no_balance_invocation_ignores_assertions() {
        let hledger = Hledger::for_tests(Path::new("/nonexistent/hledger"));
        let built = [
            // `verify_balance`
            hledger
                .invoke(["-f", "-", "balance"])
                .arg("assets:bank")
                .args(["--no-total", "--flat", "-O", "csv"]),
            // `check_assertion`
            hledger.invoke(["-f", "-", "check"]),
            // `print_json` — reads the CSV, never a journal.
            hledger
                .invoke(Vec::<&str>::new())
                .args(["print", "-f", "/j/b.csv", "--rules", "/j/b.csv.rules"])
                .args(["-O", "json"]),
        ];
        for invocation in &built {
            let argv = argv_of(invocation);
            assert!(
                !argv
                    .iter()
                    .any(|arg| arg == IGNORE_ASSERTIONS || arg == "-I"),
                "{argv:?} must NOT disable assertions"
            );
        }
    }

    // =======================================================================
    // The command line the GUI shows and the CLI runs
    // =======================================================================

    /// A dry-run request built from handles, for the renderer's tests.
    fn plan_request(balance: Option<&str>, account: Option<&str>) -> WireDryRunRequest {
        WireDryRunRequest {
            stage_id: "0123456789abcdef0123456789abcdef".to_string(),
            rules_id: "import/bank.csv.rules".to_string(),
            csv_path: "import/bank.csv".to_string(),
            journal_id: "2026/2026.journal".to_string(),
            balance: balance.map(str::to_string),
            balance_account: account.map(str::to_string),
        }
    }

    /// Re-parse a rendered argv through the **same** clap derive `ledgeline
    /// import` itself uses, so a round-trip proves the two agree rather than
    /// proving that two hand-written lists happen to match.
    fn reparse(argv: &[String]) -> CliImport {
        use clap::{Args as _, FromArgMatches as _};
        assert_eq!(
            &argv[..2],
            ["ledgeline".to_string(), "import".to_string()],
            "a rendered command must be a `ledgeline import` invocation"
        );
        let matches = CliImport::augment_args(clap::Command::new("ledgeline-import"))
            .try_get_matches_from(
                std::iter::once("ledgeline-import".to_string()).chain(argv[2..].iter().cloned()),
            )
            .expect("the rendered command line parses");
        CliImport::from_arg_matches(&matches).expect("the parsed matches rebuild the arguments")
    }

    /// The minimal run: four handles and nothing else. Every optional flag is
    /// absent, because a flag that is always printed says nothing.
    #[test]
    fn a_rendered_command_names_only_the_choices_that_were_made() {
        let request = plan_request(None, None);
        let argv = cli_argv(&CliRun {
            input: "bank.csv",
            plan: &request,
            root_journal: None,
            write_assertion: false,
            sort: false,
            dry_run: false,
            no_git: false,
        });
        assert_eq!(
            argv,
            [
                "ledgeline",
                "import",
                "-i",
                "bank.csv",
                "-o",
                "import/bank.csv",
                "-r",
                "import/bank.csv.rules",
                "-j",
                "2026/2026.journal",
            ]
        );
        assert_eq!(
            cli_invocation(&CliRun {
                input: "bank.csv",
                plan: &request,
                root_journal: None,
                write_assertion: false,
                sort: false,
                dry_run: false,
                no_git: false,
            }),
            "ledgeline import -i bank.csv -o import/bank.csv -r import/bank.csv.rules \
             -j 2026/2026.journal"
        );
    }

    /// Every choice a run can carry reaches the command line, and the root
    /// journal appears exactly when it is not the file being written to — the
    /// two-journals distinction `Plan`'s own docs are about.
    #[test]
    fn a_rendered_command_names_every_choice_including_the_second_journal() {
        let request = plan_request(Some("2949.80"), Some("assets:bank:checking"));
        let argv = cli_argv(&CliRun {
            input: "Statement.xlsx",
            plan: &request,
            root_journal: Some("main.journal"),
            write_assertion: true,
            sort: true,
            dry_run: true,
            no_git: true,
        });
        assert_eq!(
            argv,
            [
                "ledgeline",
                "import",
                "-i",
                "Statement.xlsx",
                "-o",
                "import/bank.csv",
                "-r",
                "import/bank.csv.rules",
                "-j",
                "2026/2026.journal",
                "--root-journal",
                "main.journal",
                "--balance",
                "2949.80",
                "--balance-account",
                "assets:bank:checking",
                "--write-assertion",
                "--sort",
                "--dry-run",
                "--no-git",
            ]
        );
    }

    /// The whole point of the feature: the string the GUI SHOWS parses back into
    /// the flags the CLI would RUN. One builder, both ends, proved by clap's own
    /// derive rather than by inspection.
    #[test]
    fn a_rendered_command_round_trips_through_clap() {
        let request = plan_request(Some("-3238.65"), Some("liabilities:card"));
        let run = CliRun {
            input: "statement.qfx",
            plan: &request,
            root_journal: Some("main.journal"),
            write_assertion: true,
            sort: true,
            dry_run: false,
            no_git: true,
        };
        let parsed = reparse(&cli_argv(&run));

        // The four handles come back as paths naming the same files, relative to
        // the journal's own directory — which is where the panel says to run it.
        assert_eq!(parsed.input, Path::new("statement.qfx"));
        assert_eq!(parsed.output, Path::new("import/bank.csv"));
        assert_eq!(parsed.rules, Path::new("import/bank.csv.rules"));
        assert_eq!(parsed.journal, Path::new("2026/2026.journal"));
        assert_eq!(
            parsed.root_journal.as_deref(),
            Some(Path::new("main.journal"))
        );
        assert_eq!(parsed.balance.as_deref(), Some("-3238.65"));
        assert_eq!(parsed.balance_account.as_deref(), Some("liabilities:card"));
        assert!(parsed.write_assertion);
        assert!(parsed.sort);
        assert!(!parsed.dry_run);
        assert!(parsed.no_git);

        // …and re-rendering the re-parsed run reproduces the string, so the loop
        // is closed rather than merely one-way.
        assert_eq!(
            cli_argv(&CliRun {
                input: "statement.qfx",
                plan: &request,
                root_journal: parsed.root_journal.as_deref().and_then(Path::to_str),
                write_assertion: parsed.write_assertion,
                sort: parsed.sort,
                dry_run: parsed.dry_run,
                no_git: parsed.no_git,
            }),
            cli_argv(&run)
        );
    }

    /// A name with a space in it is a real bank export (`Statement Feb 2026.xlsx`)
    /// and must survive being pasted into a shell. The argv is unquoted — it is
    /// handed to `Command::args`, which needs no quoting and must not receive
    /// any — and only the DISPLAY string carries the quotes.
    #[test]
    fn a_handle_with_a_space_is_quoted_only_for_display() {
        let request = plan_request(None, None);
        let run = CliRun {
            input: "Statement Feb 2026.xlsx",
            plan: &request,
            root_journal: None,
            write_assertion: false,
            sort: false,
            dry_run: false,
            no_git: false,
        };
        assert!(
            cli_argv(&run).contains(&"Statement Feb 2026.xlsx".to_string()),
            "argv carries the name as-is"
        );
        assert!(
            cli_invocation(&run).contains("'Statement Feb 2026.xlsx'"),
            "the display string quotes it: {}",
            cli_invocation(&run)
        );

        // An apostrophe is the one character single-quoting cannot carry, so it
        // is closed, escaped and reopened — the only shell-safe spelling.
        assert_eq!(shell_quote("Bob's bank.csv"), r"'Bob'\''s bank.csv'");
        // Nothing a shell would reinterpret is left bare.
        assert_eq!(shell_quote("a;rm -rf /"), "'a;rm -rf /'");
        // …and an ordinary handle is not decorated for nothing.
        assert_eq!(shell_quote("import/bank.csv"), "import/bank.csv");
    }

    /// The report lists are bounded, and the counts beside them are not.
    ///
    /// A user who reformatted their journal turns every row of a year's
    /// statement into a conflict, and a response body is not the place to put
    /// two thousand of them. Silently truncating would be worse than the size,
    /// so the totals are the true numbers and only the detail is clipped.
    #[test]
    fn the_id_report_clips_its_lists_but_never_its_counts() {
        let conflicts = MAX_ID_REPORTS + 5;
        let reconciliation = IdReconciliation {
            entries: String::new(),
            count: 0,
            flips: (0..conflicts)
                .map(|n| StatusFlip {
                    id: format!("FIT{n:05}"),
                    from: "pending",
                    to: "cleared",
                    new_status: Status::Cleared,
                    in_target: true,
                })
                .collect(),
            new: 0,
            unchanged: 0,
            conflicting: (0..conflicts)
                .map(|n| (format!("FIT{n:05}"), Vec::new()))
                .collect(),
        };

        let redactor = Redactor::default();
        let wire = reconciliation.wire(&redactor, true);
        assert_eq!(wire.status_changed.len(), MAX_ID_REPORTS);
        assert_eq!(
            wire.status_changed_total, conflicts,
            "the count is the truth"
        );
        assert_eq!(wire.conflicting.len(), MAX_ID_REPORTS);
        assert_eq!(wire.conflicting_total, conflicts);

        // A dry-run writes nothing, so nothing it reports was applied.
        assert!(
            reconciliation
                .wire(&redactor, false)
                .status_changed
                .iter()
                .all(|f| !f.applied)
        );
        assert!(wire.status_changed.iter().all(|f| f.applied));
    }

    /// A bank's own string comes back verbatim, and bounded.
    #[test]
    fn a_reported_value_is_the_users_own_text_clipped() {
        assert_eq!(clipped("FIT0001"), "FIT0001");
        let long = "x".repeat(MAX_FIELD_CHARS * 2);
        let bounded = clipped(&long);
        assert_eq!(bounded.chars().count(), MAX_FIELD_CHARS + 1);
        assert!(
            bounded.ends_with('\u{2026}'),
            "the clip is visible: {bounded}"
        );
        // Multi-byte characters are counted as characters, not bytes, so a
        // clip can never land inside one.
        assert_eq!(clipped("kaffee-über"), "kaffee-über");
    }
}
