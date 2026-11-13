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
//!   Unless the request sets `updateChanged`: then those ids are replaced in
//!   place with the export's version (see [`replacements`]). Off by default,
//!   because the usual reason an imported transaction differs is that its
//!   unknown accounts were categorized by hand afterwards.
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
//! # No absolute path is ever echoed
//!
//! Same rule as every other route in this crate. Nothing here reads a
//! client-supplied path at all (there is no `journalId`/`csvPath` to resolve);
//! the only paths in play are the journal's own `source_files`, and every one
//! that could reach a response goes through [`journal_handle`] (a relative,
//! forward-slash handle) first.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path as AxumPath, State};
use axum::response::Response;
use ledgeline_core::edit::InsertPosition;
use ledgeline_core::model::{
    AccountName, AliasDirective, Amount, Commodity, Journal, Posting, PostingType, SourcePos,
    Status, Tindex, Transaction,
};
use ledgeline_core::qb_import::{self, plain_aliases, resolve_account};
use ledgeline_core::qb_journal::{ImportStatus, QbJournal, QbPosting, QbTransaction};
use ledgeline_core::reimport::{self, RowClassification};
use ledgeline_core::reports::{
    AccountType, AccountTypes, KpiRole, account_decls, declared_kpi_roles, declared_types,
    kpi_resolver,
};
use ledgeline_core::sort;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use crate::AppState;
use crate::edit_api::{self, infer_style, json_body};
use crate::error::{AppError, editing_disabled};
use crate::import_api::{self, WireMove};
use crate::import_review::{StagedEdit, StagedPlace, StagedText, WireQbOverride, WireStagedTxn};
use crate::review_api::BatchEditIn;
use crate::security::no_store_json as no_store;
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
    /// Every built transaction, in export order, in the review queue's row
    /// shape (plan 32 D30) — keyed by Trans # (`id`), with its
    /// `classification` and the fingerprint an override must echo. Empty
    /// while any account is unmapped.
    txns: Vec<WireStagedTxn>,
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
pub(crate) struct WireQbIdMatches {
    // `pub(crate)`: the CLI path (`import_api::cli_import_qb_journal`) reads
    // the three counts directly off a [`commit_journal`] result rather than
    // re-deriving them — see that function's own docs.
    pub(crate) new: usize,
    pub(crate) unchanged: usize,
    conflicting: Vec<WireQbConflict>,
    pub(crate) conflicting_total: usize,
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
///
/// `pub(crate)`: also the return type of [`commit_journal`], the extracted
/// write function `import_api::cli_import_qb_journal` calls directly (see its
/// own docs) rather than going through a `stageId` this process never stages.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireQbCommit {
    pub(crate) imported: usize,
    /// Conflicting ids replaced with the export's version. Always `0` unless
    /// the request set `updateChanged`.
    pub(crate) updated: usize,
    pub(crate) id_matches: WireQbIdMatches,
    pub(crate) ordering: WireQbOrdering,
    /// How many review overrides the written transactions carry.
    pub(crate) overrides: usize,
    /// How many existing journal transactions `journalEdits` changed.
    pub(crate) journal_edited: usize,
}

/// Whether the journal is still in date order after the import, and what a
/// re-sort would move — per **touched file**, since a multi-year import can
/// land rows in more than one, unlike the CSV path's single target.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireQbOrdering {
    pub(crate) in_order: bool,
    pub(crate) files: Vec<WireQbFileOrdering>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireQbFileOrdering {
    /// A relative, forward-slash handle — the same shape `journals::targets`'
    /// ids take, and (when the file is one of the journal's own source files)
    /// usable straight away with the existing `POST /api/import/sort` route
    /// to fix it. See [`journal_handle`].
    pub(crate) journal_id: String,
    pub(crate) in_order: bool,
    moves: Vec<WireMove>,
}

/// The `commit` request body: just a stage handle. See the module docs for
/// why there is no `journalId`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireQbCommitRequest {
    stage_id: String,
    /// Replace ids the journal holds differently with the export's version,
    /// instead of only reporting them. Absent means `false`.
    #[serde(default)]
    update_changed: bool,
    /// Review edits to the export's transactions (plan 32 D30), keyed by
    /// Trans #, applied after a fingerprint check. Absent means none.
    #[serde(default)]
    overrides: Vec<WireQbOverride>,
    /// Review edits to EXISTING journal transactions (D24) — the batch
    /// route's `edits` — written in the same edit and save.
    #[serde(default)]
    journal_edits: Vec<BatchEditIn>,
}

/// The review that rides on a QuickBooks commit (plan 32 D30). Empty for the
/// command line, which has no review step.
#[derive(Default)]
pub(crate) struct QbReview<'a> {
    pub(crate) overrides: &'a [WireQbOverride],
    pub(crate) journal_edits: &'a [BatchEditIn],
}

// ===========================================================================
// Handlers
// ===========================================================================

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
    preview_of(state, id, stage.journal())
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
    // file.
    let guard = state.clone();
    let _write = guard.import_writes().lock().await;
    let Json(body) = crate::reports_api::compute(move || run_commit(&state, &request)).await?;
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

fn preview_of(
    state: &AppState,
    stage_id: String,
    parsed: &QbJournal,
) -> Result<WireQbPreview, AppError> {
    let snapshot = state.snapshot();
    let aliases = plain_aliases(&snapshot.journal);
    let unmapped = qb_import::unmapped_accounts(&aliases, &parsed.transactions);

    let (id_matches, txns) = if unmapped.is_empty() {
        let built = build_and_classify(&snapshot.journal, &aliases, &parsed.transactions)?;
        let staged = staged_text(&built)?;
        let txns = crate::import_review::wire_txns(&snapshot, &staged, None, |at| {
            let row = &built[at];
            StagedPlace::Qb(row.id.clone(), classification_word(&row.classification))
        });
        (Some(wire_id_matches(&built)), txns)
    } else {
        (None, Vec::new())
    };

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

    Ok(WireQbPreview {
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
        txns,
    })
}

/// A classification as the preview's `txns` spell it.
fn classification_word(classification: &RowClassification) -> &'static str {
    match classification {
        RowClassification::New => "new",
        RowClassification::Unchanged => "unchanged",
        // Never produced here (see `WireQbIdMatches`), and a difference is a
        // difference.
        RowClassification::StatusOnly { .. } | RowClassification::Conflicting { .. } => "changed",
    }
}

/// Every built transaction rendered exactly as the editor will write it, held
/// as one staged text (plan 32 D30) — so a fingerprint is of the text a
/// transaction would be written as, and the preview and the commit, which
/// build the same transactions, agree on it.
fn staged_text(built: &[Classified]) -> Result<StagedText, AppError> {
    let text = built
        .iter()
        .map(|row| ledgeline_core::format_transaction(&row.transaction))
        .collect::<Vec<_>>()
        .join("\n");
    StagedText::parse(&text)
}

/// Apply a commit's review overrides (plan 32 D30) to the transactions it is
/// about to write. Each must name a Trans # this commit WRITES — a new one, or
/// a changed one under `updateChanged` — and echo the fingerprint the preview
/// gave it; the edited transaction then replaces the built one.
///
/// # Errors
/// `400` for an id not in the export, or not written by this commit; `409`
/// naming the id whose fingerprint is stale; `400` for a refused op.
fn apply_overrides(
    styles: &Journal,
    built: &mut [Classified],
    overrides: &[WireQbOverride],
    update_changed: bool,
) -> Result<(), AppError> {
    if overrides.is_empty() {
        return Ok(());
    }
    crate::import_review::check_override_count(overrides.len())?;
    let mut staged = staged_text(built)?;
    let edits = overrides
        .iter()
        .map(|override_| {
            let position = built
                .iter()
                .position(|row| row.id == override_.id)
                .ok_or_else(|| {
                    AppError::BadRequest(format!(
                        "QuickBooks transaction {:?} is not in this export",
                        clip(&override_.id)
                    ))
                })?;
            let row = &built[position];
            let written = row.classification == RowClassification::New
                || (update_changed
                    && matches!(row.classification, RowClassification::Conflicting { .. }));
            if !written {
                return Err(AppError::BadRequest(format!(
                    "QuickBooks transaction {} is not written by this commit, so it cannot carry a \
                     review edit",
                    row.id
                )));
            }
            Ok(StagedEdit {
                position,
                fingerprint: &override_.fingerprint,
                ops: &override_.ops,
                label: format!(
                    "QuickBooks transaction {} ({} {})",
                    row.id, row.transaction.date, row.transaction.description
                ),
            })
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    staged.apply(&edits, styles)?;
    for edit in &edits {
        let mut edited = staged.current().transactions[edit.position].clone();
        // The editor reassigns the index and the place on write, exactly as
        // for a built transaction.
        edited.index = Tindex(0);
        edited.source_span = (
            SourcePos { line: 1, column: 1 },
            SourcePos { line: 1, column: 1 },
        );
        edited.source_file = PathBuf::new();
        built[edit.position].transaction = edited;
    }
    Ok(())
}

/// A client's string, bounded for an error body.
fn clip(value: &str) -> String {
    value.chars().take(120).collect()
}

fn wire_sample(txn: &QbTransaction) -> WireQbSample {
    WireQbSample {
        id: txn.id.clone(),
        date: txn.date.clone(),
        description: qb_import::description_for(txn),
        postings: txn
            .postings
            .iter()
            // Rendered plainly — no commodity, no style: a display string for a
            // person, not a value going into the journal (that path infers a
            // proper `AmountStyle`; see [`build_posting`]). Exact string
            // arithmetic, never a float, through the editor's own renderer: it
            // clamps `places` to `MAX_RENDER_PLACES`, which this module's private
            // copy of it did not.
            .map(|posting| {
                format!(
                    "{}  {}",
                    posting.account,
                    ledgeline_core::edit::render_dec(posting.amount, '.')
                )
            })
            .collect(),
    }
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
) -> Result<Vec<Classified>, AppError> {
    let index = reimport::build_index(journal, reimport::ID_TAG);
    let commodity = commodity_for(journal);
    let customer_lines = CustomerLines::new(journal);
    transactions
        .iter()
        .map(|qb_txn| {
            let transaction =
                build_transaction(journal, qb_txn, aliases, &commodity, &customer_lines)?;
            // Every built transaction is `Status::Unmarked` (see
            // `WireQbIdMatches`'s docs), so `status_mapped` is always `false`:
            // any status difference from a hand-marked transaction is a
            // conflict, never a silent "status-only" sync.
            let classification = reimport::classify(&index, &qb_txn.id, &transaction, false);
            Ok(Classified {
                id: qb_txn.id.clone(),
                transaction,
                classification,
            })
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
/// Preferred in order:
/// 1. The journal's own declared default (a `D AMOUNT` directive —
///    [`Journal::default_commodity`]).
/// 2. The commodity used most often across the journal's own posting amounts
///    already, first-seen order breaking a tie (the same "first occurrence"
///    precedent [`Journal::commodity_styles`]'s own docs name).
/// 3. The journal's ONE declared [`Journal::commodity_styles`] entry, if it
///    has exactly one — found necessary against a real, freshly-split
///    multi-file journal: a `commodity $1,000.00` line (hledger's DISPLAY-
///    STYLE directive — decimal places, symbol position — distinct from `D`,
///    which is the only thing step 1 reads) with no transactions anywhere
///    yet for step 2 to learn from is exactly "a journal that states, in the
///    author's own words, which commodity it is denominated in" without a
///    `D` line or existing data — the same class of signal `default_commodity`
///    exists to capture, just spelled with the OTHER directive. Two or more
///    declared commodities is genuine ambiguity, so this step declines rather
///    than guessing between them.
/// 4. No commodity at all — a journal with no default, no amount anywhere to
///    learn from, and no (or an ambiguous) declared style — which is the best
///    any answer can be there.
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
    if let Some((commodity, _)) = best {
        return commodity;
    }
    match &journal.commodity_styles[..] {
        [(commodity, _)] => commodity.clone(),
        _ => Commodity(String::new()),
    }
}

/// Which of the TARGET journal's accounts a QuickBooks line's per-line `Name`
/// becomes a `customer:` tag on (plan 37, decision 11): revenue (by resolved
/// root type), `kpi: deferred-revenue`, and receivables (declared
/// `kpi: receivable`, or the documented name fallback inside
/// [`kpi_resolver`]). Classification is by the journal's own declarations,
/// never by the QuickBooks account's name, so a vendor on an expense line or an
/// employee on a payroll line is left exactly as it was.
///
/// Built once per batch: both maps are read off the journal's `account`
/// directives, and the type resolution is memoized per account name.
struct CustomerLines {
    roles: std::collections::BTreeMap<String, KpiRole>,
    types: AccountTypes,
}

impl CustomerLines {
    fn new(journal: &Journal) -> Self {
        Self {
            roles: declared_kpi_roles(&journal.accounts),
            types: AccountTypes::from_declared(declared_types(&account_decls(journal))),
        }
    }

    /// True when a line posting to `account` should carry its `Name` as the
    /// customer.
    fn takes_customer(&self, account: &str) -> bool {
        self.types.is_type(account, AccountType::Revenue)
            || matches!(
                kpi_resolver(&self.roles, &self.types)(account),
                Some(KpiRole::DeferredRevenue | KpiRole::Receivable)
            )
    }
}

fn build_transaction(
    journal: &Journal,
    qb_txn: &QbTransaction,
    aliases: &[&AliasDirective],
    commodity: &Commodity,
    customer_lines: &CustomerLines,
) -> Result<Transaction, AppError> {
    let postings: Vec<Posting> = qb_txn
        .postings
        .iter()
        .map(|posting| build_posting(journal, posting, aliases, commodity, customer_lines))
        .collect();
    let postings = merge_same_account_postings(&qb_txn.id, postings)?;
    Ok(Transaction {
        // Placeholder; the editor reassigns file-order indices on reparse —
        // the same convention `edit_api::build_transaction` follows.
        index: Tindex(0),
        date: qb_txn.date.clone(),
        date2: None,
        status: match qb_txn.status {
            ImportStatus::Unmarked => Status::Unmarked,
            ImportStatus::Pending => Status::Pending,
            ImportStatus::Cleared => Status::Cleared,
        },
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
    })
}

/// Combine postings that resolved to the SAME account and carry an IDENTICAL
/// memo and `class`/`customer`/`vendor` tags, summing their amounts.
///
/// QuickBooks can legitimately split one account's real effect across
/// several rows of one Journal Entry — a payroll entry touching `…:Benefits`
/// three times, one of them `$0.00`, is a real, reported shape. Repeated
/// postings to the same account are valid hledger on their own (they simply
/// sum for balance purposes), but writing them out separately means writing
/// an EXPLICIT `0.00` whenever rows to the same account happen to cancel —
/// and a transaction with more than one such explicit zero is one stray
/// external reformat away from hledger's "there can't be more than one real
/// posting with no amount" refusal (an hledger-based format-on-save tool
/// doing exactly that, turning `0.00` into a blank/elided amount, is what
/// surfaced this). Merging first removes the zero rows this way rather than
/// just writing around them.
///
/// memo and class/customer/vendor must match EXACTLY (confirmed with the
/// user, since this is real financial data): two postings to the same account
/// with a DIFFERING memo or tag — already folded into each
/// [`Posting::comment`] by [`build_posting`]/[`posting_comment`] by the time
/// this runs, so comparing the rendered comment IS comparing all four — are
/// kept separate rather than merged, so no per-line detail is ever silently
/// combined or dropped. Order-preserving: a merged posting stays at its
/// FIRST occurrence's position. Since only rows with identical comments
/// merge, there is no "whose comment wins" question.
///
/// That includes a `customer:` DERIVED from the line's own `Name` (plan 37):
/// it is written into the comment before this runs, so two revenue lines for
/// different customers can never be summed into one.
///
/// # Errors
/// [`AppError::BadRequest`], naming `id`, if summing two real QuickBooks
/// amounts overflows `Dec`'s `i128` mantissa — not reachable by any
/// real-world dollar figure, but every other arithmetic step in this
/// pipeline (`qb_journal::close`'s own balance sum chief among them) refuses
/// rather than panics on the same, so this does too.
fn merge_same_account_postings(id: &str, postings: Vec<Posting>) -> Result<Vec<Posting>, AppError> {
    let mut merged: Vec<Posting> = Vec::with_capacity(postings.len());
    for posting in postings {
        let existing = merged
            .iter_mut()
            .find(|other| other.account == posting.account && other.comment == posting.comment);
        match existing {
            Some(target) => {
                let summed = target.amounts[0]
                    .quantity
                    .add(posting.amounts[0].quantity)
                    .map_err(|_| {
                        AppError::BadRequest(format!(
                            "transaction {id}: merging repeated postings to {} overflowed",
                            posting.account.0
                        ))
                    })?;
                target.amounts[0].quantity = summed;
            }
            None => merged.push(posting),
        }
    }
    Ok(merged)
}

fn build_posting(
    journal: &Journal,
    posting: &QbPosting,
    aliases: &[&AliasDirective],
    commodity: &Commodity,
    customer_lines: &CustomerLines,
) -> Posting {
    let account = resolve_account(&posting.account, aliases)
        .expect("caller guarantees every account resolves before building a transaction");
    let name_is_customer = customer_lines.takes_customer(&account);
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
        comment: posting_comment(posting, name_is_customer),
        tags: Vec::new(),
        generated: None,
        elided: false,
    }
}

/// `class`/`customer`/`vendor` folded into the posting comment as tags — per
/// direct instruction (see `plans/17-quickbooks-journal-import.md`), preserved
/// rather than discarded even though hledger has no equivalent field for any
/// of the three.
///
/// Each value goes through [`qb_import::journal_safe`] first. A `;` here is
/// harmless to the comment boundary itself (this text always lands after the
/// line's OWN leading `; `, so the first `;` `split_comment` finds is already
/// spent) but a raw newline is not — it would still split this posting's one
/// line into two — so the same sanitizer used for the description is reused
/// rather than reasoning about which of its two replacements this call site
/// still needs.
///
/// The row's own `memo` (QuickBooks' per-line Description) leads the comment
/// as plain note text, ahead of the tags. It was once left out entirely, and
/// that was a real data-loss bug, not a style choice: a nine-line revenue
/// recognition Journal Entry with one customer per line and one description per
/// line built eighteen postings whose comments were all empty. So
/// [`merge_same_account_postings`] summed them into one debit and one credit,
/// and nothing in the written journal said which customer accounted for which
/// part of the total. With the memo in the comment, rows with different memos
/// can never merge.
///
/// Every TAG value is made safe twice over: [`qb_import::journal_safe`] for
/// the line itself, then [`tag_value`] because hledger ends a tag value at a
/// comma (`customer: Acme, Inc.` reads as `Acme`).
///
/// `name_is_customer` (see [`CustomerLines`]): the line posts to a revenue,
/// deferred-revenue or receivable account, so when it has no Customer column
/// value its own `Name` cell is written as `customer:` instead. A Customer
/// column value always wins.
/// A QuickBooks value made safe as an hledger TAG value: journal-safe, with
/// every comma turned into a space and runs of spaces collapsed, so
/// `Acme, Inc.` is written (and read back by hledger) as `Acme Inc.`.
fn tag_value(value: &str) -> String {
    qb_import::journal_safe(value)
        .replace(',', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn posting_comment(posting: &QbPosting, name_is_customer: bool) -> String {
    let memo = posting
        .memo
        .as_deref()
        .filter(|memo| !memo.trim().is_empty())
        .map(|memo| qb_import::journal_safe(memo.trim()));
    let customer = posting
        .customer
        .as_deref()
        .or_else(|| posting.name.as_deref().filter(|_| name_is_customer));
    let tags = [
        ("class", posting.class.as_deref()),
        ("customer", customer),
        ("vendor", posting.vendor.as_deref()),
    ]
    .into_iter()
    .filter_map(|(name, value)| value.map(|value| format!("{name}: {}", tag_value(value))));
    let segments: Vec<String> = memo.into_iter().chain(tags).collect();
    if segments.is_empty() {
        String::new()
    } else {
        format!("{}\n", segments.join(", "))
    }
}

// ===========================================================================
// Commit: refuse unmapped accounts, write, report
// ===========================================================================

fn run_commit(state: &AppState, request: &WireQbCommitRequest) -> Result<WireQbCommit, AppError> {
    let (_, stage) = resolve_stage(state, &request.stage_id)?;
    commit_journal(
        state,
        stage.journal(),
        request.update_changed,
        &QbReview {
            overrides: &request.overrides,
            journal_edits: &request.journal_edits,
        },
    )
}

/// Classify every parsed transaction by id, refuse if any account is
/// unmapped, otherwise build and write the new ones, and report ordering.
///
/// This is [`run_commit`]'s own body from "unmapped accounts block the
/// write" onward (see the plan's Phase D amendments,
/// `plans/17-quickbooks-journal-import.md`), factored out so it takes a
/// [`QbJournal`] **directly** instead of a `stageId` — the CLI
/// (`import_api::cli_import_qb_journal`) has no stage at all, it reads the
/// file and parses it right there, so a `stageId`-shaped entry point would
/// force it to fake one. `run_commit` above is now a thin `resolve_stage` +
/// call to this. There is exactly one write path: nothing downstream of the
/// unmapped-accounts check is duplicated anywhere else in this crate.
pub(crate) fn commit_journal(
    state: &AppState,
    parsed: &QbJournal,
    update_changed: bool,
    review: &QbReview<'_>,
) -> Result<WireQbCommit, AppError> {
    crate::review_api::check_edits(review.journal_edits)?;
    let (snapshot, mut built) = classify_parsed(state, parsed)?;
    let id_matches = wire_id_matches(&built);
    apply_overrides(&snapshot.raw, &mut built, review.overrides, update_changed)?;
    let (new, changed): (Vec<Classified>, Vec<Classified>) = built
        .into_iter()
        .filter(|row| {
            row.classification == RowClassification::New
                || (update_changed
                    && matches!(row.classification, RowClassification::Conflicting { .. }))
        })
        .partition(|row| row.classification == RowClassification::New);
    let changed: Vec<(String, Transaction)> = changed
        .into_iter()
        .map(|row| (row.id, row.transaction))
        .collect();
    // The QuickBooks Trans #, date and description of each — so a write
    // failure names the one transaction it happened on rather than an index
    // into a batch that can run into the thousands. See
    // `edit_api::add_and_replace_transactions`'s own docs on why this has to be supplied
    // here: nothing past this point still knows which QuickBooks id a given
    // `Transaction` came from.
    let labels: Vec<String> = new
        .iter()
        .map(|row| {
            format!(
                "{} — {} {}",
                row.id, row.transaction.date, row.transaction.description
            )
        })
        .collect();
    let new_transactions: Vec<Transaction> = new.into_iter().map(|row| row.transaction).collect();

    if new_transactions.is_empty()
        && replacements(&snapshot.journal, &changed).is_empty()
        && review.journal_edits.is_empty()
    {
        return Ok(WireQbCommit {
            imported: 0,
            updated: 0,
            id_matches,
            ordering: qb_ordering(&snapshot, &[]),
            overrides: 0,
            journal_edited: 0,
        });
    }

    let (touched, updated) = edit_api::add_and_replace_transactions(
        state,
        review.journal_edits,
        &new_transactions,
        &labels,
        InsertPosition::DateOrdered,
        &|journal| replacements(journal, &changed),
    )?;

    let imported = new_transactions.len();

    // `edit_api::add_and_replace_transactions` already republished the snapshot; read it
    // fresh here so the ordering check sees the bytes it just wrote.
    let ordering = qb_ordering(&state.snapshot(), &touched);

    Ok(WireQbCommit {
        imported,
        updated,
        id_matches,
        ordering,
        overrides: review.overrides.len(),
        journal_edited: review.journal_edits.len(),
    })
}

/// Which of `changed` — `(QuickBooks id, transaction built from the export)`
/// for each conflicting id — to write over the journal's copy, and with what.
///
/// The export's version wins on everything QuickBooks owns: date,
/// description, postings, accounts, amounts, memos. Four fields it never
/// exports are kept from the journal's copy instead, so updating cannot
/// wipe them: the status (`*`/`!` from reconciling), the code, the secondary
/// date, and the header comment (which carries the `id:` tag and any note
/// written beside it).
///
/// An id carried by more than one transaction is skipped; there is no way to
/// say which copy the export meant. So is one whose only difference was in
/// those four kept fields, because the replacement would be identical.
///
/// Resolved by id against `journal` every time rather than once, so the
/// caller can ask again after additions have moved every index (see
/// [`edit_api::add_and_replace_transactions`]).
fn replacements(
    journal: &Journal,
    changed: &[(String, Transaction)],
) -> Vec<edit_api::Replacement> {
    let index = reimport::build_index(journal, reimport::ID_TAG);
    changed
        .iter()
        .filter_map(|(id, incoming)| {
            let (existing, 1) = index.get(id)? else {
                return None;
            };
            let transaction = Transaction {
                index: existing.index,
                status: existing.status,
                code: existing.code.clone(),
                date2: existing.date2.clone(),
                comment: existing.comment.clone(),
                ..incoming.clone()
            };
            matches!(
                reimport::classify(&index, id, &transaction, false),
                RowClassification::Conflicting { .. }
            )
            .then(|| edit_api::Replacement {
                index: existing.index,
                label: format!("{id} — {} {}", transaction.date, transaction.description),
                transaction,
            })
        })
        .collect()
}

/// [`classify_report`]'s result: the three id-match counts, plus every
/// conflicting id and what differs about it.
///
/// The detail is there because a bare "1 conflicting" told a person nothing
/// they could act on. A transaction QuickBooks changed after it was first
/// imported (a line added to a Journal Entry, say) is left alone by design,
/// so the journal keeps the old figures, and the only place the difference
/// was ever reported was a count.
pub(crate) struct QbClassifyReport {
    pub(crate) new: usize,
    pub(crate) unchanged: usize,
    pub(crate) conflicting: usize,
    pub(crate) conflicts: Vec<import_api::CliQbConflict>,
    /// How many conflicts an `updateChanged` commit would replace — see
    /// [`replacements`] for the ones it would not.
    pub(crate) updatable: usize,
}

/// The read-only half of [`commit_journal`]: classify `parsed`'s transactions
/// by id against `state`'s current journal, refusing (naming every unmapped
/// account, via [`unmapped_refusal`]) exactly as a commit would, but never
/// writing anything.
///
/// Used by the CLI's `--dry-run` for a QuickBooks Journal input
/// (`import_api::cli_import_qb_journal`), mirroring the CSV path's own "the
/// dry run always happens" rule (see `import_api::cli_import`'s doc comment)
/// rather than threading a `write: bool` through [`commit_journal`]: this
/// function's own work — an alias scan and an in-memory classify, no
/// subprocess, no I/O beyond `state.snapshot()` — costs nothing worth
/// avoiding, so a committing CLI run classifies twice (once here for the
/// report, once inside `commit_journal` for the write) exactly as a
/// committing CSV run pays for a real `hledger import --dry-run` subprocess
/// it does not strictly need either. The alternative (a `write` flag on
/// `commit_journal`) would leave that function's own contract — "the part of
/// `run_commit` from 'unmapped accounts block the write' onward", which
/// always writes — no longer accurately described by its name, and would let
/// a future caller pass `write: false` to the HTTP commit handler by mistake.
pub(crate) fn classify_report(
    state: &AppState,
    parsed: &QbJournal,
) -> Result<QbClassifyReport, AppError> {
    let (snapshot, built) = classify_parsed(state, parsed)?;
    let matches = wire_id_matches(&built);
    let changed: Vec<(String, Transaction)> = built
        .iter()
        .filter(|row| matches!(row.classification, RowClassification::Conflicting { .. }))
        .map(|row| (row.id.clone(), row.transaction.clone()))
        .collect();
    let updatable = replacements(&snapshot.journal, &changed).len();
    let conflicts = built
        .into_iter()
        .filter_map(|row| match row.classification {
            RowClassification::Conflicting { diffs, .. } => {
                Some(import_api::CliQbConflict { id: row.id, diffs })
            }
            _ => None,
        })
        .collect();
    Ok(QbClassifyReport {
        new: matches.new,
        unchanged: matches.unchanged,
        conflicting: matches.conflicting_total,
        conflicts,
        updatable,
    })
}

/// The refusal-or-classify step [`commit_journal`]/[`classify_report`] both
/// start from: an alias scan, then either the [`unmapped_refusal`] (naming
/// every account with no matching alias) or [`build_and_classify`]'s result.
/// The one place this check's wording is written, so both callers — and both
/// the HTTP and CLI doors reaching them — refuse in the exact same words.
fn classify_parsed(
    state: &AppState,
    parsed: &QbJournal,
) -> Result<(Arc<crate::Snapshot>, Vec<Classified>), AppError> {
    let snapshot = state.snapshot();
    let aliases = plain_aliases(&snapshot.journal);
    let unmapped = qb_import::unmapped_accounts(&aliases, &parsed.transactions);
    if !unmapped.is_empty() {
        return Err(unmapped_refusal(&unmapped));
    }
    let built = build_and_classify(&snapshot.journal, &aliases, &parsed.transactions)?;
    Ok((snapshot, built))
}

/// "These QuickBooks accounts have no matching alias…" — written once so the
/// HTTP `commit` route and the CLI's non-interactive refusal (which has
/// nobody to prompt, so it names the same accounts and stops — see the
/// plan's Phase D section) say the identical sentence.
fn unmapped_refusal(unmapped: &[String]) -> AppError {
    AppError::BadRequest(format!(
        "these QuickBooks accounts have no matching alias, so nothing was written: {}. Add a \
         plain alias for each one (PUT /api/aliases/{{journalId}}) and try again.",
        unmapped.join(", ")
    ))
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
pub(crate) fn journal_handle(journal: &Journal, path: &Path) -> String {
    journal_id_for(journal, path).unwrap_or_else(|| {
        path.file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned())
    })
}

/// Whether each file the import touched is still in date order, read back
/// through the journal's vault (`snapshot`'s storage). A file that cannot be
/// read back is left out rather than failing an import that already landed.
fn qb_ordering(snapshot: &crate::Snapshot, touched: &[PathBuf]) -> WireQbOrdering {
    let journal = &snapshot.journal;
    let storage = snapshot.storage();
    let mut files = Vec::new();
    let mut in_order = true;
    for path in touched {
        let Some(text) = storage.as_ref().and_then(|location| {
            let file = location.path_of(path)?;
            location.vault().read_to_string(&file).ok()
        }) else {
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
    use ledgeline_core::{Dec, parse_journal};

    fn journal(text: &str) -> Journal {
        parse_journal(text, "t.journal").expect("the fixture parses")
    }

    #[test]
    fn a_sample_amount_renders_through_the_clamped_renderer() {
        let posting = |amount| QbPosting {
            account: "1000 Checking".to_string(),
            amount,
            memo: None,
            class: None,
            customer: None,
            vendor: None,
            name: None,
        };
        let txn = QbTransaction {
            id: "1".to_string(),
            date: "2026-01-01".to_string(),
            transaction_type: "Deposit".to_string(),
            num: None,
            name: None,
            postings: vec![posting(Dec::new(-1250, 2)), posting(Dec::new(1, u32::MAX))],
            status: ledgeline_core::qb_journal::ImportStatus::Unmarked,
        };
        let sample = wire_sample(&txn);
        assert_eq!(sample.postings[0], "1000 Checking  -12.50");
        // `"0".repeat(places)` unclamped asked for four gigabytes here.
        assert!(
            sample.postings[1].len() < 300,
            "{}",
            sample.postings[1].len()
        );
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

    /// The real report: a freshly-split multi-file journal whose main file
    /// declares `commodity $1,000.00` (hledger's DISPLAY-STYLE directive) but
    /// has no `D` line and, because the split just happened, no transactions
    /// anywhere yet for the frequency fallback to learn from.
    #[test]
    fn a_lone_declared_commodity_style_wins_when_there_is_nothing_else_to_go_on() {
        let journal = journal("commodity $1,000.00\n");
        assert_eq!(journal.default_commodity, None);
        assert!(journal.transactions.is_empty());
        assert_eq!(
            commodity_for(&journal),
            Commodity("$".to_string()),
            "one declared commodity, with nothing else to prefer, is unambiguous"
        );
    }

    #[test]
    fn two_declared_commodity_styles_with_nothing_else_to_go_on_stays_bare() {
        let journal = journal("commodity $1,000.00\ncommodity EUR 1.000,00\n");
        assert!(journal.transactions.is_empty());
        assert_eq!(
            commodity_for(&journal),
            Commodity(String::new()),
            "two declared commodities is genuine ambiguity, not a signal to guess from"
        );
    }

    // -----------------------------------------------------------------------
    // merge_same_account_postings
    // -----------------------------------------------------------------------

    fn dollar_style() -> ledgeline_core::model::AmountStyle {
        ledgeline_core::model::AmountStyle {
            side: ledgeline_core::model::CommoditySide::Left,
            spaced: false,
            decimal_mark: Some('.'),
            digit_groups: None,
            precision: 2,
        }
    }

    fn posting(account: &str, comment: &str, cents: i128) -> Posting {
        Posting {
            status: Status::Unmarked,
            ptype: PostingType::Regular,
            account: AccountName(account.to_string()),
            amounts: vec![Amount {
                commodity: Commodity("$".to_string()),
                quantity: ledgeline_core::Dec::new(cents, 2),
                style: dollar_style(),
                cost: None,
            }],
            balance_assertion: None,
            date: None,
            date2: None,
            comment: comment.to_string(),
            tags: Vec::new(),
            generated: None,
            elided: false,
        }
    }

    fn cents_of(posting: &Posting) -> i128 {
        posting.amounts[0].quantity.mantissa
    }

    #[test]
    fn repeated_postings_to_the_same_account_with_matching_tags_merge_and_sum() {
        // The exact reported shape: three "Benefits" rows, one of them $0.00.
        let postings = vec![
            posting("expenses:benefits", "", 608702),
            posting("expenses:benefits", "", 0),
            posting("expenses:benefits", "", 13451),
        ];
        let merged = merge_same_account_postings("913", postings).expect("no overflow");
        assert_eq!(merged.len(), 1, "all three collapse into one posting");
        assert_eq!(cents_of(&merged[0]), 622153);
    }

    #[test]
    fn postings_with_differing_tags_are_never_merged() {
        let postings = vec![
            posting("expenses:benefits", "vendor: Acme", 10000),
            posting("expenses:benefits", "vendor: Beta", 5000),
        ];
        let merged = merge_same_account_postings("1", postings).expect("no overflow");
        assert_eq!(
            merged.len(),
            2,
            "differing class/customer/vendor keeps postings separate, confirmed with the user"
        );
    }

    #[test]
    fn a_merge_group_that_sums_to_zero_still_writes_an_explicit_posting() {
        let postings = vec![
            posting("expenses:payroll-tax", "", 216795),
            posting("expenses:payroll-tax", "", -216795),
        ];
        let merged = merge_same_account_postings("1", postings).expect("no overflow");
        assert_eq!(merged.len(), 1);
        assert_eq!(
            cents_of(&merged[0]),
            0,
            "a net-zero merge stays visible as an explicit posting, not dropped"
        );
    }

    #[test]
    fn merging_preserves_first_occurrence_order_and_leaves_unrelated_accounts_alone() {
        // Mirrors the reported transaction's own shape: Wages, Benefits,
        // PayrollTax, Benefits(0), PayrollTax(0), Wages, Benefits.
        let postings = vec![
            posting("assets:checking", "", -3639330),
            posting("expenses:wages", "", 2709939),
            posting("expenses:benefits", "", 608702),
            posting("expenses:payroll-tax", "", 216795),
            posting("expenses:benefits", "", 0),
            posting("expenses:payroll-tax", "", 0),
            posting("expenses:wages", "", 90443),
            posting("expenses:benefits", "", 13451),
        ];
        let merged = merge_same_account_postings("913", postings).expect("no overflow");
        let accounts: Vec<&str> = merged.iter().map(|p| p.account.0.as_str()).collect();
        assert_eq!(
            accounts,
            [
                "assets:checking",
                "expenses:wages",
                "expenses:benefits",
                "expenses:payroll-tax"
            ],
            "one posting per account, in first-occurrence order"
        );
        let by_account: std::collections::HashMap<&str, i128> = merged
            .iter()
            .map(|p| (p.account.0.as_str(), cents_of(p)))
            .collect();
        assert_eq!(by_account["assets:checking"], -3639330);
        assert_eq!(by_account["expenses:wages"], 2709939 + 90443);
        assert_eq!(by_account["expenses:benefits"], 608702 + 13451);
        assert_eq!(by_account["expenses:payroll-tax"], 216795);
        let sum: i128 = merged.iter().map(cents_of).sum();
        assert_eq!(
            sum, 0,
            "the merge changes nothing about the transaction's own balance"
        );
    }

    // -----------------------------------------------------------------------
    // posting_comment: the per-line memo
    // -----------------------------------------------------------------------

    fn qb_posting(account: &str, cents: i128, memo: Option<&str>) -> QbPosting {
        QbPosting {
            account: account.to_string(),
            amount: Dec::new(cents, 2),
            memo: memo.map(str::to_string),
            class: None,
            customer: None,
            vendor: None,
            name: None,
        }
    }

    #[test]
    fn the_memo_leads_the_posting_comment() {
        let posting = qb_posting(
            "2500 Deferred Revenue",
            1750000,
            Some("Recognized Zephyr Revenue"),
        );
        assert_eq!(
            posting_comment(&posting, false),
            "Recognized Zephyr Revenue\n"
        );
    }

    #[test]
    fn the_memo_comes_before_the_tags() {
        let mut posting = qb_posting("2500 Deferred Revenue", 100, Some("Annual license"));
        posting.customer = Some("Acme".to_string());
        posting.class = Some("Enterprise".to_string());
        assert_eq!(
            posting_comment(&posting, false),
            "Annual license, class: Enterprise, customer: Acme\n"
        );
    }

    #[test]
    fn a_blank_memo_writes_no_comment() {
        assert_eq!(
            posting_comment(&qb_posting("1000 Checking", 100, Some("  ")), false),
            ""
        );
        assert_eq!(
            posting_comment(&qb_posting("1000 Checking", 100, None), false),
            ""
        );
    }

    #[test]
    fn a_memo_is_made_journal_safe() {
        let posting = qb_posting("1000 Checking", 100, Some("Smith; Jones\nretainer"));
        assert_eq!(posting_comment(&posting, false), "Smith, Jones retainer\n");
    }

    // -----------------------------------------------------------------------
    // per-line Name -> customer: (plan 37)
    // -----------------------------------------------------------------------

    #[test]
    fn every_tag_value_loses_its_commas() {
        let mut posting = qb_posting("4000 Sales", -100, Some("Smith, Jones"));
        posting.class = Some("East, North".to_string());
        posting.customer = Some("Acme, Inc.".to_string());
        posting.vendor = Some("Hooli ,LLC".to_string());
        // The memo is free text, not a tag: its comma stays.
        assert_eq!(
            posting_comment(&posting, false),
            "Smith, Jones, class: East North, customer: Acme Inc., vendor: Hooli LLC\n"
        );
        posting.customer = None;
        posting.name = Some("Globex, Ltd.".to_string());
        assert!(posting_comment(&posting, true).contains("customer: Globex Ltd.,"));
    }

    #[test]
    fn a_name_becomes_the_customer_only_when_the_line_takes_one() {
        let mut posting = qb_posting("4000 Sales", -100, None);
        posting.name = Some("Acme; Corp".to_string());
        assert_eq!(posting_comment(&posting, true), "customer: Acme Corp\n");
        assert_eq!(posting_comment(&posting, false), "");
        // A Customer column value always wins over the Name cell.
        posting.customer = Some("Globex".to_string());
        assert_eq!(posting_comment(&posting, true), "customer: Globex\n");
    }

    #[test]
    fn customer_lines_classify_by_the_target_journals_declarations() {
        let journal = journal(
            "account income:subs      ; type: R\n\
             account liabilities:unearned  ; type: L, kpi: deferred-revenue\n\
             account liabilities:loan  ; type: L\n\
             account assets:trade-ar   ; type: A, kpi: receivable\n\
             account assets:accounts receivable  ; type: A\n\
             account assets:bank       ; type: C\n\
             account costs:payroll     ; type: X\n\
             account sales             ; type: X\n",
        );
        let lines = CustomerLines::new(&journal);
        for (account, expected) in [
            ("income:subs:smb", true),
            ("liabilities:unearned", true),
            ("assets:trade-ar", true),
            ("assets:accounts receivable", true),
            ("liabilities:loan", false),
            ("assets:bank", false),
            ("costs:payroll", false),
            // Named like revenue, declared an expense: the declaration wins.
            ("sales", false),
        ] {
            assert_eq!(lines.takes_customer(account), expected, "{account}");
        }
    }

    #[test]
    fn same_account_lines_naming_different_customers_never_merge() {
        let journal = journal(
            "account revenue:sales  ; type: R\n\
             alias 2500 Deferred Revenue = liabilities:deferred-revenue\n\
             alias 4000 Sales = revenue:sales\n\
             account liabilities:deferred-revenue  ; type: L, kpi: deferred-revenue\n\
             2026-01-01 opening\n    assets:bank  $1000.00\n    equity:opening\n",
        );
        let aliases = plain_aliases(&journal);
        let line = |account: &str, cents: i128, name: &str| {
            let mut posting = qb_posting(account, cents, None);
            posting.name = Some(name.to_string());
            posting
        };
        let qb_txn = QbTransaction {
            id: "77".to_string(),
            date: "2026-02-28".to_string(),
            transaction_type: "Journal Entry".to_string(),
            num: None,
            name: Some("Acme".to_string()),
            postings: vec![
                line("2500 Deferred Revenue", 10000, "Acme"),
                line("4000 Sales", -10000, "Acme"),
                line("2500 Deferred Revenue", 5000, "Globex"),
                line("4000 Sales", -5000, "Globex"),
                line("2500 Deferred Revenue", 2500, "Acme"),
                line("4000 Sales", -2500, "Acme"),
            ],
            status: ledgeline_core::qb_journal::ImportStatus::Unmarked,
        };
        let built = build_transaction(
            &journal,
            &qb_txn,
            &aliases,
            &commodity_for(&journal),
            &CustomerLines::new(&journal),
        )
        .expect("builds");
        let written: Vec<(&str, i128, &str)> = built
            .postings
            .iter()
            .map(|p| (p.account.0.as_str(), cents_of(p), p.comment.as_str()))
            .collect();
        assert_eq!(
            written,
            [
                ("liabilities:deferred-revenue", 12500, "customer: Acme\n"),
                ("revenue:sales", -12500, "customer: Acme\n"),
                ("liabilities:deferred-revenue", 5000, "customer: Globex\n"),
                ("revenue:sales", -5000, "customer: Globex\n"),
            ]
        );
    }

    /// The reported shape: one Journal Entry recognizing revenue for several
    /// customers, every line debiting the same deferred-revenue account and
    /// crediting the same sales account, with the customer named only in each
    /// line's memo. It was written as one debit and one credit, which made the
    /// per-customer split, and any later change to it, impossible to see.
    #[test]
    fn same_account_lines_with_different_memos_stay_separate_postings() {
        let journal = journal(
            "alias 2500 Deferred Revenue = liabilities:deferred-revenue\n\
             alias 4000 Sales = revenue:sales\n\
             2026-01-01 opening\n    assets:bank  $1000.00\n    equity:opening\n",
        );
        let aliases = plain_aliases(&journal);
        let lines = [
            ("Recognized Hoover Revenue", 3040875),
            ("Recognized Zephyr Revenue", 1750000),
            ("Attago", 910000),
        ];
        let postings = lines
            .iter()
            .flat_map(|(memo, cents)| {
                [
                    qb_posting("2500 Deferred Revenue", *cents, Some(memo)),
                    qb_posting(
                        "4000 Sales:4400 Hybrid Cloud Enterprise",
                        -cents,
                        Some(memo),
                    ),
                ]
            })
            .collect();
        let qb_txn = QbTransaction {
            id: "10972".to_string(),
            date: "2026-07-31".to_string(),
            transaction_type: "Journal Entry".to_string(),
            num: Some("598".to_string()),
            name: None,
            postings,
            status: ledgeline_core::qb_journal::ImportStatus::Unmarked,
        };
        let built = build_transaction(
            &journal,
            &qb_txn,
            &aliases,
            &commodity_for(&journal),
            &CustomerLines::new(&journal),
        )
        .expect("builds");
        let written: Vec<(&str, i128, &str)> = built
            .postings
            .iter()
            .map(|p| (p.account.0.as_str(), cents_of(p), p.comment.as_str()))
            .collect();
        assert_eq!(
            written,
            [
                (
                    "liabilities:deferred-revenue",
                    3040875,
                    "Recognized Hoover Revenue\n"
                ),
                (
                    "revenue:sales:4400 Hybrid Cloud Enterprise",
                    -3040875,
                    "Recognized Hoover Revenue\n"
                ),
                (
                    "liabilities:deferred-revenue",
                    1750000,
                    "Recognized Zephyr Revenue\n"
                ),
                (
                    "revenue:sales:4400 Hybrid Cloud Enterprise",
                    -1750000,
                    "Recognized Zephyr Revenue\n"
                ),
                ("liabilities:deferred-revenue", 910000, "Attago\n"),
                (
                    "revenue:sales:4400 Hybrid Cloud Enterprise",
                    -910000,
                    "Attago\n"
                ),
            ]
        );
    }

    #[test]
    fn same_account_lines_with_the_same_memo_still_merge() {
        let postings = vec![
            posting("expenses:benefits", "March payroll\n", 608702),
            posting("expenses:benefits", "March payroll\n", 0),
            posting("expenses:benefits", "March payroll\n", 13451),
        ];
        let merged = merge_same_account_postings("913", postings).expect("no overflow");
        assert_eq!(merged.len(), 1);
        assert_eq!(cents_of(&merged[0]), 622153);
    }
}
