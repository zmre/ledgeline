//! The HTTP surface for the QuickBooks Online **Journal** report import
//! (WP-17 Phase B; see `plans/17-quickbooks-journal-import.md`).
//!
//! This is a second import pipeline beside `import_api`'s CSV/OFX one — the
//! report is multi-row-per-transaction and already double-entry, so it cannot
//! go through `hledger import` at all (see `ledgeline_core::qb_journal`'s
//! module docs). Nothing here runs a subprocess. A parsed export is turned
//! into real [`Transaction`]s and written with
//! [`JournalEditor::add_transaction`](ledgeline_core::edit::JournalEditor::add_transaction),
//! the same capability the manual "add transaction" editor uses.
//!
//! Three routes, and only the last two live here:
//!
//! - `POST /api/import/stage` (an existing route, modified in `import_api.rs`:
//!   [`qb_journal::detect`] is checked before the ordinary CSV/spreadsheet
//!   dispatch, and `import_api::stage_qb_journal` parses eagerly and stages
//!   the result in [`QbStageArea`] — see that function's own docs for why the
//!   *parsed* [`QbJournal`] is what gets staged rather than the raw bytes).
//! - `GET /api/import/qb-journal/{stageId}` ([`preview`]) — re-report a staged
//!   upload's parsed groups and which accounts are still unmapped, without
//!   writing anything. A client calls this again after adding an alias
//!   through the *existing* `PUT /api/aliases/{*journalId}` route (see the
//!   plan's "narrow alias exception" — this module does not grow a second way
//!   to write one).
//! - `POST /api/import/qb-journal/commit` ([`commit`]) — write. Refuses while
//!   any account is unmapped, naming them; otherwise writes every id the
//!   journal does not already hold, skips ids it holds identically, and
//!   reports (never overwrites) ids it holds differently — the same "ask,
//!   don't guess" and "a hand-edit outranks a re-download" policies the CSV
//!   path's [`reimport`] already encodes, reused rather than re-derived.
//!
//! # No `journalId`
//!
//! CSV import writes to exactly one file the user names, because
//! `hledger import` is pointed at one target. This pipeline writes through
//! [`JournalEditor::add_transaction`] with
//! [`InsertPosition::DateOrdered`](ledgeline_core::edit::InsertPosition::DateOrdered),
//! which already decides — per transaction, from the journal's own chronology
//! — which `include`d file receives each row. There is no single destination
//! to name, so the commit request below carries only a `stageId`.
//!
//! # The git safety net, and why it checks the whole tree
//!
//! CSV import's git check (`import_api::blocked_by_git`) looks at the one or
//! two files it is about to write, known in advance from the request. A
//! multi-year QuickBooks export can route its rows into any number of
//! `include`d files, and which ones is not known until after the write. So the
//! PRE-write check here looks at every file [`Journal::source_files`] lists —
//! the superset of anything `DateOrdered` could possibly touch — and the
//! POST-write git commit is then narrowed to exactly the files
//! [`edit_api::add_transactions`] reports as touched
//! ([`JournalEditor::dirty_files`](ledgeline_core::edit::JournalEditor::dirty_files)),
//! so the commit's blast radius matches what was actually written, same as
//! the CSV path's own `committed` set.
//!
//! # No absolute path is ever echoed
//!
//! Same rule as every other route in this crate. Nothing here reads a
//! client-supplied path at all (there is no `journalId`/`csvPath` to resolve);
//! the only paths in play are the journal's own `source_files`, and every one
//! that could reach a response goes through [`journal_handle`] (a relative,
//! forward-slash handle) or [`import_api::Redactor`] first.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path as AxumPath, State};
use axum::http::{HeaderName, HeaderValue, header};
use axum::response::{IntoResponse, Response};
use ledgeline_core::edit::InsertPosition;
use ledgeline_core::model::{
    AccountName, AliasDirective, Amount, Commodity, Journal, Posting, PostingType, SourcePos,
    Status, Tindex, Transaction,
};
use ledgeline_core::qb_import::{self, plain_aliases, resolve_account};
use ledgeline_core::qb_journal::{QbJournal, QbPosting, QbTransaction};
use ledgeline_core::reimport::{self, RowClassification};
use ledgeline_core::sort;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use crate::AppState;
use crate::edit_api::{self, infer_style, json_body};
use crate::error::{AppError, editing_disabled};
use crate::import_api::{self, GitPolicy, Redactor, WireGitResult, WireMove};
use crate::stage::StageId;

/// How many staged QuickBooks Journal uploads one session keeps, oldest first.
///
/// The same reasoning as `stage::MAX_LIVE_STAGES`: nobody legitimately drops
/// more than a couple of exports without committing one, and this bounds how
/// many parsed exports a client that never commits can make this process
/// hold.
const MAX_LIVE_QB_STAGES: usize = 8;

/// How many parsed transactions one preview response samples. Same number as
/// `import_api::PREVIEW_ROWS`, for the same reason: enough to recognise your
/// own data, not enough to turn a big export into a browser-choking body.
const SAMPLE_TRANSACTIONS: usize = 20;

// ===========================================================================
// Staging: the parsed export, in memory (WP-17 Phase B)
// ===========================================================================

/// One staged QuickBooks Journal upload: its already-parsed
/// [`QbJournal`], held in memory.
///
/// Unlike `stage::Stage`, which holds a CSV `hledger` must read from a real
/// file, nothing here is ever handed to a subprocess: this pipeline never
/// shells out. `import_api::stage_qb_journal` parses once, at upload time
/// (so a truncated export is refused immediately, by name, rather than staged
/// and refused later), and every route below reads the same parse — there is
/// no second reader that could come to disagree with the first about what the
/// bytes mean.
#[derive(Debug)]
pub(crate) struct QbStage {
    journal: QbJournal,
}

impl QbStage {
    /// The parsed export.
    pub(crate) fn journal(&self) -> &QbJournal {
        &self.journal
    }
}

/// The per-session staging area for QuickBooks Journal uploads. One per
/// [`AppState`], shared by every clone of it — see `stage::StageArea`'s own
/// docs for why this has to be per-session.
#[derive(Debug, Default)]
pub(crate) struct QbStageArea {
    inner: Mutex<Vec<(StageId, Arc<QbStage>)>>,
}

impl QbStageArea {
    /// Stage `journal` and hand back its handle, or `None` if the OS CSPRNG
    /// refused to mint one (see `StageId::mint`).
    pub(crate) fn put(&self, journal: QbJournal) -> Option<(StageId, Arc<QbStage>)> {
        let id = StageId::mint()?;
        let stage = Arc::new(QbStage { journal });
        let mut area = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        area.push((id.clone(), Arc::clone(&stage)));
        // Evict from the front, so the stage a user is actively working with
        // (always the newest) is the last one to go — same rule
        // `stage::StageArea::put` follows.
        while area.len() > MAX_LIVE_QB_STAGES {
            area.remove(0);
        }
        Some((id, stage))
    }

    /// The stage `id` names, or `None`. The only id → stage resolution, by
    /// exact equality against a map this process built — see `StageId`'s own
    /// docs for why that is the whole of the security argument.
    pub(crate) fn get(&self, id: &StageId) -> Option<Arc<QbStage>> {
        let area = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        area.iter()
            .find(|(known, _)| known == id)
            .map(|(_, stage)| Arc::clone(stage))
    }
}

// ===========================================================================
// Wire types
// ===========================================================================

/// `GET /api/import/qb-journal/{stageId}` — a staged export's parsed groups,
/// its date-format guess, and which accounts are still unmapped.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireQbPreview {
    stage_id: String,
    transaction_count: usize,
    posting_count: usize,
    date_format: WireQbDateFormat,
    /// Distinct QuickBooks account names no plain alias in the journal maps,
    /// in first-seen order. Non-empty blocks a commit — see the module docs.
    unmapped_accounts: Vec<String>,
    /// A clipped sample of the parsed transactions, so a person can recognise
    /// their own data.
    sample: Vec<WireQbSample>,
    /// What a commit would do right now, classified by id against the
    /// journal. `null` while any account is unmapped: nothing can be built
    /// (and therefore nothing classified) without a resolved account.
    id_matches: Option<WireQbIdMatches>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireQbDateFormat {
    format: String,
    ambiguous: bool,
}

/// One parsed transaction, flattened for display — the same shape
/// `import_api::WireProposed` uses for a CSV candidate's sample.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireQbSample {
    id: String,
    date: String,
    description: String,
    postings: Vec<String>,
}

/// What matching this export's transactions against the journal by id found —
/// the QuickBooks-import analogue of `import_api::WireIdMatches`. No
/// `statusChanged` list: a built transaction is always
/// [`Status::Unmarked`] (nothing in a QuickBooks export maps to hledger's
/// clearing status), so [`reimport::classify`] can never answer
/// [`RowClassification::StatusOnly`] here — seeing one would mean a status
/// difference, and that is folded into `conflicting`, same as the CSV path
/// when its own rules file assigns no status.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireQbIdMatches {
    new: usize,
    unchanged: usize,
    conflicting: Vec<WireQbConflict>,
    conflicting_total: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireQbConflict {
    id: String,
    diffs: Vec<WireQbFieldDiff>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireQbFieldDiff {
    field: String,
    existing: String,
    incoming: String,
}

/// `POST /api/import/qb-journal/commit` — what was written.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireQbCommit {
    imported: usize,
    id_matches: WireQbIdMatches,
    ordering: WireQbOrdering,
    /// `null` when nothing was written, when no touched file is under version
    /// control, or when the git safety net is off.
    git: Option<WireGitResult>,
}

/// Whether the journal is still in date order after the import, and what a
/// re-sort would move — per **touched file**, since a multi-year import can
/// land rows in more than one, unlike the CSV path's single target.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireQbOrdering {
    in_order: bool,
    files: Vec<WireQbFileOrdering>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireQbFileOrdering {
    /// A relative, forward-slash handle — the same shape `journals::targets`'
    /// ids take, and (when the file is one of the journal's own source files)
    /// usable straight away with the existing `POST /api/import/sort` route
    /// to fix it. See [`journal_handle`].
    journal_id: String,
    in_order: bool,
    moves: Vec<WireMove>,
}

/// The `commit` request body: just a stage handle. See the module docs for
/// why there is no `journalId`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireQbCommitRequest {
    stage_id: String,
}

// ===========================================================================
// Handlers
// ===========================================================================

/// `Cache-Control: no-store` — the same posture as every other `/api/import/*`
/// route; nothing here is derived from the journal snapshot's generation
/// counter.
fn no_store<T: Serialize>(body: T) -> Response {
    const NO_STORE: (HeaderName, HeaderValue) =
        (header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    ([NO_STORE], Json(body)).into_response()
}

/// `GET /api/import/qb-journal/{stageId}`.
pub(crate) async fn preview(
    State(state): State<AppState>,
    AxumPath(stage_id): AxumPath<String>,
) -> Result<Response, AppError> {
    let Json(body) = crate::reports_api::compute(move || run_preview(&state, &stage_id)).await?;
    Ok(no_store(body))
}

fn run_preview(state: &AppState, stage_id: &str) -> Result<WireQbPreview, AppError> {
    let (id, stage) = resolve_stage(state, stage_id)?;
    Ok(preview_of(state, id, stage.journal()))
}

/// `POST /api/import/qb-journal/commit`.
pub(crate) async fn commit(
    State(state): State<AppState>,
    payload: Result<Json<WireQbCommitRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    let request = json_body(payload)?;
    if !state.editing_enabled() {
        return Err(editing_disabled());
    }
    // Serialize against CSV imports too: both can touch the same journal
    // file, and both may run a git commit over it — see the module docs.
    let guard = state.clone();
    let _write = guard.import_writes().lock().await;
    let Json(body) =
        crate::reports_api::compute(move || run_commit(&state, &request.stage_id)).await?;
    Ok(no_store(body))
}

// ===========================================================================
// Resolving a stage
// ===========================================================================

fn resolve_stage(state: &AppState, raw: &str) -> Result<(String, Arc<QbStage>), AppError> {
    let id = StageId::parse(raw).ok_or_else(|| stage_not_found(raw))?;
    let stage = state
        .qb_stages()
        .get(&id)
        .ok_or_else(|| stage_not_found(raw))?;
    Ok((id.as_str().to_string(), stage))
}

fn stage_not_found(raw: &str) -> AppError {
    let clipped: String = raw.chars().take(120).collect();
    AppError::NotFound(format!(
        "{clipped:?} is not a staged QuickBooks Journal upload this server is holding; drop the \
         file again"
    ))
}

// ===========================================================================
// Preview: classify against the journal as it stands, report
// ===========================================================================

fn preview_of(state: &AppState, stage_id: String, parsed: &QbJournal) -> WireQbPreview {
    let snapshot = state.snapshot();
    let aliases = plain_aliases(&snapshot.journal);
    let unmapped = qb_import::unmapped_accounts(&aliases, &parsed.transactions);

    let id_matches = unmapped.is_empty().then(|| {
        let built = build_and_classify(&snapshot.journal, &aliases, &parsed.transactions);
        wire_id_matches(&built)
    });

    let posting_count = parsed
        .transactions
        .iter()
        .map(|txn| txn.postings.len())
        .sum();
    let sample = parsed
        .transactions
        .iter()
        .take(SAMPLE_TRANSACTIONS)
        .map(wire_sample)
        .collect();

    WireQbPreview {
        stage_id,
        transaction_count: parsed.transactions.len(),
        posting_count,
        date_format: WireQbDateFormat {
            format: parsed.date_format.format.clone(),
            ambiguous: parsed.date_format.ambiguous,
        },
        unmapped_accounts: unmapped,
        sample,
        id_matches,
    }
}

fn wire_sample(txn: &QbTransaction) -> WireQbSample {
    WireQbSample {
        id: txn.id.clone(),
        date: txn.date.clone(),
        description: qb_import::description_for(txn),
        postings: txn
            .postings
            .iter()
            .map(|posting| format!("{}  {}", posting.account, render_dec(posting.amount)))
            .collect(),
    }
}

/// `Dec` rendered plainly for a preview sample — no commodity, no style: this
/// is a display string for a person, not a value going into the journal (that
/// path infers a proper [`ledgeline_core::model::AmountStyle`]; see
/// [`build_posting`]). Exact string arithmetic, never a float — the same
/// reason `qb_journal`'s own module never prints a cell's stored value
/// through one.
fn render_dec(amount: ledgeline_core::Dec) -> String {
    let sign = if amount.mantissa < 0 { "-" } else { "" };
    let digits = amount.mantissa.unsigned_abs().to_string();
    let places = amount.places as usize;
    if places == 0 {
        return format!("{sign}{digits}");
    }
    let padded = if digits.len() > places {
        digits
    } else {
        format!("{}{digits}", "0".repeat(places - digits.len() + 1))
    };
    let split = padded.len() - places;
    format!("{sign}{}.{}", &padded[..split], &padded[split..])
}

/// One classified transaction: its QuickBooks id, the [`Transaction`] built
/// from it (every account already resolved), and what [`reimport::classify`]
/// made of it against the journal as it stands.
struct Classified {
    id: String,
    transaction: Transaction,
    classification: RowClassification,
}

/// Build every transaction and classify it by id, in export order.
///
/// # Preconditions
/// Every posting account in `transactions` must resolve against `aliases` —
/// i.e. [`qb_import::unmapped_accounts`] over the same two arguments must be
/// empty. Callers check that first (see [`preview_of`]/[`run_commit`]) because
/// there is nothing safe to build otherwise; asking this function to guess an
/// account would be exactly the "guess or default" this pipeline was built to
/// refuse.
fn build_and_classify(
    journal: &Journal,
    aliases: &[&AliasDirective],
    transactions: &[QbTransaction],
) -> Vec<Classified> {
    let index = reimport::build_index(journal, reimport::ID_TAG);
    let commodity = commodity_for(journal);
    transactions
        .iter()
        .map(|qb_txn| {
            let transaction = build_transaction(journal, qb_txn, aliases, &commodity);
            // Every built transaction is `Status::Unmarked` (see
            // `WireQbIdMatches`'s docs), so `status_mapped` is always `false`:
            // any status difference from a hand-marked transaction is a
            // conflict, never a silent "status-only" sync.
            let classification = reimport::classify(&index, &qb_txn.id, &transaction, false);
            Classified {
                id: qb_txn.id.clone(),
                transaction,
                classification,
            }
        })
        .collect()
}

fn wire_id_matches(built: &[Classified]) -> WireQbIdMatches {
    let mut new = 0;
    let mut unchanged = 0;
    let mut conflicting = Vec::new();
    for row in built {
        match &row.classification {
            RowClassification::New => new += 1,
            RowClassification::Unchanged => unchanged += 1,
            // A QuickBooks-built proposal never assigns a status, so this
            // never occurs (see `WireQbIdMatches`'s docs) — matched anyway,
            // exhaustively, rather than behind a wildcard that would hide a
            // change to `classify`'s status-mapping rule.
            RowClassification::StatusOnly { .. } => {}
            RowClassification::Conflicting { diffs, .. } => {
                conflicting.push(WireQbConflict {
                    id: row.id.clone(),
                    diffs: diffs
                        .iter()
                        .map(|diff| WireQbFieldDiff {
                            field: diff.field.clone(),
                            existing: diff.existing.clone(),
                            incoming: diff.incoming.clone(),
                        })
                        .collect(),
                });
            }
        }
    }
    let conflicting_total = conflicting.len();
    WireQbIdMatches {
        new,
        unchanged,
        conflicting,
        conflicting_total,
    }
}

// ===========================================================================
// Building a `core::Transaction` from one `QbTransaction`
// ===========================================================================

/// The commodity every posting [`build_posting`] writes, since a QuickBooks
/// export carries no currency information of its own at all — every amount is
/// a bare number.
///
/// Preferred in order: the journal's own declared default (a `D AMOUNT`
/// directive — [`Journal::default_commodity`]); else the commodity used most
/// often across the journal's own posting amounts already, first-seen order
/// breaking a tie (the same "first occurrence" precedent
/// [`Journal::commodity_styles`]'s own docs name); else — a journal with no
/// default and no amount anywhere to learn from — no commodity at all, which
/// is the best any answer can be there.
///
/// **Not** `default_commodity` alone. Most real journals write `$100.00`
/// throughout and never declare a `D` line (the scratch journal
/// `qb_journal_endpoints.rs` tests against is exactly this shape), so falling
/// back to an empty commodity on `None` would write every QuickBooks-imported
/// amount with NO commodity symbol — silently a different journal style than
/// the rest of the file, and exactly the "blank commodity" failure mode
/// `edit_api`'s balance-assertion check already refuses for the same reason:
/// it does not round-trip to the amount that was meant.
fn commodity_for(journal: &Journal) -> Commodity {
    if let Some(commodity) = &journal.default_commodity {
        return commodity.clone();
    }
    let mut counts: Vec<(Commodity, usize)> = Vec::new();
    for txn in &journal.transactions {
        for posting in &txn.postings {
            for amount in &posting.amounts {
                match counts
                    .iter_mut()
                    .find(|(seen, _)| seen == &amount.commodity)
                {
                    Some((_, count)) => *count += 1,
                    None => counts.push((amount.commodity.clone(), 1)),
                }
            }
        }
    }
    let mut best: Option<(Commodity, usize)> = None;
    for (commodity, count) in counts {
        let beats_current = best.as_ref().is_none_or(|(_, current)| count > *current);
        if beats_current {
            best = Some((commodity, count));
        }
    }
    best.map_or_else(|| Commodity(String::new()), |(commodity, _)| commodity)
}

fn build_transaction(
    journal: &Journal,
    qb_txn: &QbTransaction,
    aliases: &[&AliasDirective],
    commodity: &Commodity,
) -> Transaction {
    let postings = qb_txn
        .postings
        .iter()
        .map(|posting| build_posting(journal, posting, aliases, commodity))
        .collect();
    Transaction {
        // Placeholder; the editor reassigns file-order indices on reparse —
        // the same convention `edit_api::build_transaction` follows.
        index: Tindex(0),
        date: qb_txn.date.clone(),
        date2: None,
        status: Status::Unmarked,
        code: String::new(),
        description: qb_import::description_for(qb_txn),
        // The re-import tag convention `reimport::ID_TAG` already reads.
        comment: format!("id: {}\n", qb_txn.id),
        preceding_comment: String::new(),
        tags: Vec::new(),
        postings,
        source_span: (
            SourcePos { line: 1, column: 1 },
            SourcePos { line: 1, column: 1 },
        ),
        source_file: PathBuf::new(),
    }
}

fn build_posting(
    journal: &Journal,
    posting: &QbPosting,
    aliases: &[&AliasDirective],
    commodity: &Commodity,
) -> Posting {
    let account = resolve_account(&posting.account, aliases)
        .expect("caller guarantees every account resolves before building a transaction");
    let style = infer_style(journal, commodity, posting.amount.places);
    Posting {
        status: Status::Unmarked,
        ptype: PostingType::Regular,
        account: AccountName(account),
        amounts: vec![Amount {
            commodity: commodity.clone(),
            quantity: posting.amount,
            style,
            cost: None,
        }],
        balance_assertion: None,
        date: None,
        date2: None,
        comment: posting_comment(posting),
        tags: Vec::new(),
    }
}

/// `class`/`customer`/`vendor` folded into the posting comment as tags — per
/// direct instruction (see `plans/17-quickbooks-journal-import.md`), preserved
/// rather than discarded even though hledger has no equivalent field for any
/// of the three.
fn posting_comment(posting: &QbPosting) -> String {
    let tags: Vec<String> = [
        ("class", posting.class.as_deref()),
        ("customer", posting.customer.as_deref()),
        ("vendor", posting.vendor.as_deref()),
    ]
    .into_iter()
    .filter_map(|(name, value)| value.map(|value| format!("{name}: {value}")))
    .collect();
    if tags.is_empty() {
        String::new()
    } else {
        format!("{}\n", tags.join(", "))
    }
}

// ===========================================================================
// Commit: refuse unmapped accounts, git-check, write, git-commit, report
// ===========================================================================

fn run_commit(state: &AppState, stage_id: &str) -> Result<WireQbCommit, AppError> {
    let (_, stage) = resolve_stage(state, stage_id)?;
    let parsed = stage.journal();

    let snapshot = state.snapshot();
    let aliases = plain_aliases(&snapshot.journal);
    let unmapped = qb_import::unmapped_accounts(&aliases, &parsed.transactions);
    if !unmapped.is_empty() {
        return Err(AppError::BadRequest(format!(
            "these QuickBooks accounts have no matching alias, so nothing was written: {}. Add a \
             plain alias for each one (PUT /api/aliases/{{journalId}}) and try again.",
            unmapped.join(", ")
        )));
    }

    let built = build_and_classify(&snapshot.journal, &aliases, &parsed.transactions);
    let id_matches = wire_id_matches(&built);
    let new_transactions: Vec<Transaction> = built
        .into_iter()
        .filter(|row| row.classification == RowClassification::New)
        .map(|row| row.transaction)
        .collect();

    if new_transactions.is_empty() {
        return Ok(WireQbCommit {
            imported: 0,
            id_matches,
            ordering: qb_ordering(&snapshot.journal, &[]),
            git: None,
        });
    }

    // Sequencing rule (mirrors `import_api::run_commit`'s rule 3): checked
    // server-side, over every file this write *could* touch — see the module
    // docs for why that is the whole tree rather than one known target.
    let prefs = crate::prefs::load();
    let handles: Vec<String> = snapshot
        .journal
        .source_files
        .iter()
        .map(|path| journal_handle(&snapshot.journal, path))
        .collect();
    let git_targets: Vec<(&Path, &str)> = snapshot
        .journal
        .source_files
        .iter()
        .map(PathBuf::as_path)
        .zip(handles.iter().map(String::as_str))
        .collect();
    let blocked = if GitPolicy::FromPrefs.enabled(&prefs) {
        import_api::blocked_by_git(&git_targets)
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

    let touched =
        edit_api::add_transactions(state, &new_transactions, InsertPosition::DateOrdered)?;

    let touched_handles: Vec<String> = touched
        .iter()
        .map(|path| journal_handle(&snapshot.journal, path))
        .collect();
    let redactor = touched
        .iter()
        .zip(touched_handles.iter())
        .fold(Redactor::default(), |redactor, (path, handle)| {
            redactor.hide(path, handle)
        });
    let committed: Vec<(&Path, &str)> = touched
        .iter()
        .map(PathBuf::as_path)
        .zip(touched_handles.iter().map(String::as_str))
        .collect();
    let imported = new_transactions.len();
    let git = GitPolicy::FromPrefs
        .enabled(&prefs)
        .then(|| import_api::commit_targets(&committed, &commit_message(imported), &redactor))
        .filter(|result| result.committed || result.message.is_some());

    // `edit_api::add_transactions` already republished the snapshot; read it
    // fresh here so the ordering check sees the bytes it just wrote.
    let ordering = qb_ordering(&state.snapshot().journal, &touched);

    Ok(WireQbCommit {
        imported,
        id_matches,
        ordering,
        git,
    })
}

fn commit_message(imported: usize) -> String {
    let plural = if imported == 1 { "" } else { "s" };
    format!("import {imported} transaction{plural} from a QuickBooks Journal export")
}

/// A journal id for `path`: relative to the main file's own directory,
/// forward-slash separated — the same shape `journals::targets` produces
/// (`journals::relative_id`'s algorithm, not re-exported, so reproduced here
/// rather than widening that module's public surface for one caller).
/// `None` when `path` is not below the journal's own directory.
fn journal_id_for(journal: &Journal, path: &Path) -> Option<String> {
    let root = journal.source_files.first()?.parent()?;
    let relative = path.strip_prefix(root).ok()?;
    let parts: Option<Vec<&str>> = relative
        .components()
        .map(|component| match component {
            std::path::Component::Normal(name) => name.to_str(),
            _ => None,
        })
        .collect();
    parts
        .filter(|parts| !parts.is_empty())
        .map(|parts| parts.join("/"))
}

/// [`journal_id_for`], falling back to `path`'s bare file name when it is not
/// below the journal's own directory (should not happen for anything in
/// [`Journal::source_files`], but a display handle must never be an empty
/// string).
fn journal_handle(journal: &Journal, path: &Path) -> String {
    journal_id_for(journal, path).unwrap_or_else(|| {
        path.file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned())
    })
}

fn qb_ordering(journal: &Journal, touched: &[PathBuf]) -> WireQbOrdering {
    let mut files = Vec::new();
    let mut in_order = true;
    for path in touched {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let (file_in_order, moves) = match sort::plan(&text) {
            Ok(plan) => (
                plan.unchanged,
                plan.moves.iter().map(WireMove::from).collect(),
            ),
            // A journal `sort::plan` will not touch (a yearless date) is still
            // a journal this import just wrote into successfully; report it as
            // in order rather than failing an import that has already landed.
            Err(_) => (true, Vec::new()),
        };
        in_order &= file_in_order;
        files.push(WireQbFileOrdering {
            journal_id: journal_handle(journal, path),
            in_order: file_in_order,
            moves,
        });
    }
    WireQbOrdering { in_order, files }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ledgeline_core::parse_journal;

    fn journal(text: &str) -> Journal {
        parse_journal(text, "t.journal").expect("the fixture parses")
    }

    /// The scratch journal `qb_journal_endpoints.rs` tests against, and the
    /// ordinary shape of a real journal: `$` amounts throughout, no `D`
    /// directive anywhere. `default_commodity` is `None` here, and the ONLY
    /// evidence of what this journal is denominated in is its own postings.
    #[test]
    fn with_no_default_directive_the_journals_own_dollar_amounts_win() {
        let journal = journal(
            "2026-01-01 opening balances\n    assets:bank:checking  $1000.00\n    equity:opening\n",
        );
        assert_eq!(journal.default_commodity, None);
        assert_eq!(commodity_for(&journal), Commodity("$".to_string()));
    }

    #[test]
    fn a_declared_default_commodity_wins_even_over_a_more_frequent_one() {
        let journal = journal(
            "D EUR 1,000.00\n\
             2026-01-01 a\n    x  EUR1.00\n    y\n\n\
             2026-01-02 b\n    x  $1.00\n    y\n\n\
             2026-01-03 c\n    x  $1.00\n    y\n",
        );
        assert_eq!(
            commodity_for(&journal),
            Commodity("EUR".to_string()),
            "the declared default outranks a commodity used more often"
        );
    }

    #[test]
    fn with_no_default_and_no_amounts_anywhere_there_is_no_commodity_to_prefer() {
        let journal = journal("2026-01-01 a\n    x  1\n    y  -1\n");
        assert_eq!(commodity_for(&journal), Commodity(String::new()));
    }

    #[test]
    fn a_tied_frequency_is_broken_by_first_occurrence() {
        let journal = journal(
            "2026-01-01 a\n    x  $1.00\n    y\n\n\
             2026-01-02 b\n    x  EUR1.00\n    y\n",
        );
        assert_eq!(
            commodity_for(&journal),
            Commodity("$".to_string()),
            "both commodities appear once; the one seen first wins"
        );
    }
}
