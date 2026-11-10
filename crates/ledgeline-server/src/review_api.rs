//! The transaction-review routes (plan 32 Phase 2): the two review queues,
//! similar transactions, the payee and tag vocabularies, and the batch write
//! that saves a review card together with every similar transaction it was
//! applied to. The model lives in [`ledgeline_core::review`]; this module is
//! routing, caching and transport, like [`crate::journal_api`].
//!
//! # Routes
//!
//! - `GET /api/review/queue?kind=unknowns|recents&today&limit&maxDays&offset`
//!   — one page of a queue, every item a full row (D14: stepping between cards
//!   never waits on the network) with its fingerprint, similarity key and
//!   suggestions.
//! - `GET /api/review/item/{index}?today` — one transaction as a queue item,
//!   for the card's single-transaction mode (D31).
//! - `GET /api/review/similar/{index}?today&queue=1,2,3` — the unreviewed
//!   transactions sharing `index`'s similarity key, the precedent among the
//!   categorised ones, and which candidates are in the client's queue.
//! - `GET /api/review/similar-key?key&today` — the same for a similarity key
//!   with no journal transaction behind it (a staged import's, plan 32 D24).
//! - `GET /api/review/suggest?description&today` — a queue item's
//!   suggestions for a description alone, for the add form (D36).
//! - `GET /api/payees`, `GET /api/tags` — autocomplete vocabularies (D15),
//!   one body per snapshot.
//! - `POST /api/transactions/batch` — `{edits: [{index, fingerprint, ops}]}`,
//!   applied as ONE edit: one lock, one re-parse, one save (D14).
//!
//! # Fingerprints
//!
//! Every transaction these routes hand out carries `fingerprint`: an opaque
//! token for the exact source text it was read from
//! ([`ledgeline_core::Fingerprint::token`] over
//! [`JournalEditor::transaction_fingerprints`]). The batch echoes it back, and
//! a transaction whose text changed since — another tab's save, an external
//! editor — is a `409` naming it rather than an edit to text nobody saw.
//! Clients compare tokens for equality only; the format is not a contract.
//!
//! The editor holds the source text, so fingerprints are read from it, under
//! its lock, together with the snapshot the rows come from: every publish
//! happens under the same lock, so the pair always describes one journal. A
//! server with no editor (read-only) answers `fingerprint: null`.
//!
//! # Rows and markers
//!
//! Every review transaction is the `/api/journal/rows` row shape plus two
//! markers ([`ReviewRow`]), which the journal rows themselves now carry too
//! (D20):
//!
//! - `unknown` — the transaction is in the Unknowns sense of D3/D4: a posting
//!   to an uncategorised account (one an `=` rule generated included — its
//!   `generated` names the rule), and not `reviewed:ignore`;
//! - `reviewed` — it carries a `reviewed` tag of any value, `ignore`
//!   included, so it is not "unreviewed" (D1).
//!
//! # Posting positions in a batch
//!
//! `setPostingAccount.posting` counts the postings the FILE writes — the
//! review form's rows — not the served postings, which with `--auto` in force
//! include the generated ones (each carries `generated`). A generated posting
//! therefore cannot be addressed at all. `PATCH /api/transactions/{index}`
//! counts served postings instead; the batch follows the engine's own
//! [`EditOp::SetPostingAccount`].

use crate::edit_api::{
    AddRequest, COMMENT, MAX_CODE_BYTES, MAX_COMMENT_BYTES, MAX_DATE_BYTES, MAX_DESCRIPTION_BYTES,
    NONE, StatusIn, TAB, bound, build_transaction, check_text, json_body, lock_editor, redacted,
    refuse_generated, save_and_publish,
};
use crate::error::AppError;
use crate::journal_api::{
    DiagnosticsEntry, conditional, dated_etag, flags_of, not_modified, ok_json, position_of,
    to_bytes,
};
use crate::reports_api::{self, parse_date, today_utc};
use crate::{AppState, Snapshot, generation_of, if_none_match_hits};
use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use ledgeline_core::model::{Journal, Tindex};
use ledgeline_core::periodic::MAX_ACCOUNT_BYTES;
use ledgeline_core::review::{
    self, QueueKind, QueueOpts, ReviewIndex, ReviewState, Suggestion, SuggestionSource, is_unknown,
    similarity_key,
};
use ledgeline_core::view::rows::{Row, RowContext};
use ledgeline_core::{EditError, EditOp, Fingerprint};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;

/// The largest queue page a request may ask for. Recents default to 50 and
/// Unknowns page at 100 (D4); a client raising its limit gets at most this.
const MAX_QUEUE_PAGE: usize = 500;

/// The most transactions one batch may edit: a card plus every similar
/// candidate ([`review::SIMILAR_CAP`]), with room to spare.
pub(crate) const MAX_BATCH: usize = 1_000;

/// The most indices `?queue=` may list — a generous bound on a review run, so
/// a hostile URL cannot make the server build an enormous set.
const MAX_QUEUE_INDICES: usize = 10_000;

/// The longest tag name a batch may write. Tag names are words; this only
/// keeps a hostile body from writing a megabyte one.
const MAX_TAG_NAME_BYTES: usize = 256;

// ---------------------------------------------------------------------------
// Rows with review markers (D20)
// ---------------------------------------------------------------------------

/// A journal row plus its review markers: what `/api/journal/rows`,
/// `/api/transactions/{index}` and every route here serve. See the module
/// docs for what `unknown` and `reviewed` mean.
#[derive(Serialize)]
pub(crate) struct ReviewRow<'a> {
    #[serde(flatten)]
    row: Row<'a>,
    unknown: bool,
    reviewed: bool,
}

impl<'a> ReviewRow<'a> {
    /// `row` (the transaction at `position`) with the markers `index` holds
    /// for it.
    pub(crate) fn at(index: &ReviewIndex, position: usize, row: Row<'a>) -> Self {
        let state = index.state_at(position);
        Self::marked(row, state, index.unknown_at(position))
    }

    /// `row` with the markers for a transaction in `state` that is, or is
    /// not, [`is_unknown`]. `pub(crate)`: an import's staged transactions
    /// (plan 32 Phase 4) are in no review index, so they are judged directly.
    pub(crate) fn marked(row: Row<'a>, state: Option<&ReviewState>, unknown: bool) -> Self {
        Self {
            row,
            unknown: unknown && state != Some(&ReviewState::Ignored),
            reviewed: state.is_some_and(ReviewState::is_tagged),
        }
    }
}

/// A row as a review route serves it: a [`ReviewRow`] plus the fingerprint a
/// batch must echo back to edit it.
#[derive(Serialize)]
pub(crate) struct ReviewTxn<'a> {
    #[serde(flatten)]
    pub(crate) row: ReviewRow<'a>,
    pub(crate) fingerprint: Option<String>,
}

// ---------------------------------------------------------------------------
// The summary's review counts (D18)
// ---------------------------------------------------------------------------

/// The client's date and review settings, for counting the queues.
pub(crate) struct ReviewWindow {
    /// The client's local date (default: today, UTC).
    pub(crate) today: String,
    /// Recents' cap (default 50, D4).
    limit: usize,
    /// The shared lookback window in days, `0` for no limit (default 365).
    max_days: u32,
}

/// `review` in `GET /api/journal/summary`: the Journal toolbar's badges.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReviewCounts {
    /// How many editable transactions Review Unknowns would queue.
    unknown: usize,
    /// How many Review Recents would queue: its total, capped at `limit`.
    unreviewed_recent: usize,
    /// How many more unknowns qualify but live in a file that cannot be
    /// written.
    not_editable: usize,
    /// The same for Recents (the two overlap, so they are not summed).
    not_editable_recent: usize,
}

impl ReviewWindow {
    /// `today`, `reviewLimit` and `reviewMaxDays` as a request spelled them,
    /// defaulting to [`QueueOpts::new`]'s D4 values.
    pub(crate) fn resolve(
        today: Option<String>,
        limit: Option<usize>,
        max_days: Option<u32>,
    ) -> Result<Self, AppError> {
        let today = parse_date("today", today, today_utc)?;
        let defaults = QueueOpts::new(QueueKind::Recents, &today);
        let (limit, max_days) = (
            limit.unwrap_or(defaults.limit),
            max_days.unwrap_or(defaults.max_days),
        );
        Ok(Self {
            today,
            limit,
            max_days,
        })
    }

    /// Both queues' sizes over `journal`.
    ///
    /// Plan 32 amendment 13, as Patrick decided it (amendment A): a posting an
    /// `=` rule generated on a real transaction counts. If it posts to an
    /// unknown account the transaction is unknown here AND in the Problems
    /// drawer's `uncategorized` check, so the badge and the drawer agree.
    /// Forecast transactions (`~` rules, and any `=` postings on them) count
    /// nowhere — the review index and the drawer both skip them. Such an
    /// unknown can only be fixed in its rule (its generated posting is
    /// read-only on the card); each served posting's `generated` names the
    /// rule.
    pub(crate) fn counts(&self, journal: &Journal, index: &ReviewIndex) -> ReviewCounts {
        let opts = |kind| QueueOpts {
            kind,
            today: &self.today,
            limit: 0,
            max_days: self.max_days,
            offset: 0,
        };
        let unknowns = index.queue(journal, &opts(QueueKind::Unknowns));
        let recents = index.queue(journal, &opts(QueueKind::Recents));
        ReviewCounts {
            unknown: unknowns.total,
            unreviewed_recent: recents.total.min(self.limit),
            not_editable: unknowns.not_editable,
            not_editable_recent: recents.not_editable,
        }
    }
}

// ---------------------------------------------------------------------------
// Reading rows and fingerprints together
// ---------------------------------------------------------------------------

/// What [`fingerprinted`] read: the snapshot, the transactions picked from it
/// with their fingerprints (same order), and whatever else the pick found.
pub(crate) struct Picked<T> {
    pub(crate) snapshot: Arc<Snapshot>,
    pub(crate) indices: Vec<Tindex>,
    pub(crate) fingerprints: Vec<Option<String>>,
    pub(crate) found: T,
}

/// The published snapshot, the transactions `select` picks from it, and their
/// fingerprints — read under the editor lock, so the rows and the
/// fingerprints describe the same journal (see the module docs).
///
/// The lock is held only for the pick and the fingerprint scan; the rows are
/// built after it is released.
pub(crate) fn fingerprinted<T>(
    state: &AppState,
    select: impl FnOnce(&Snapshot) -> Result<(Vec<Tindex>, T), AppError>,
) -> Result<Picked<T>, AppError> {
    let guard = lock_editor(state)?;
    let snapshot = state.snapshot();
    let (indices, found) = select(&snapshot)?;
    let fingerprints = match guard.as_ref() {
        // The editor's journal is the snapshot's raw one unless a publish
        // somehow bypassed the lock; then no token is better than a wrong one.
        Some(editor) if Arc::ptr_eq(editor.journal(), &snapshot.raw) => editor
            .transaction_fingerprints(&indices)
            .iter()
            .map(|fingerprint| fingerprint.as_ref().map(Fingerprint::token))
            .collect(),
        _ => vec![None; indices.len()],
    };
    drop(guard);
    Ok(Picked {
        snapshot,
        indices,
        fingerprints,
        found,
    })
}

/// Answer a review GET: `304` when the client already holds this snapshot's
/// body for this URL and date, else `job` on the blocking pool, tagged with
/// the snapshot it actually read (which a concurrent edit may have made newer
/// than the one the `304` check saw).
pub(crate) async fn answer<F>(
    state: AppState,
    headers: &HeaderMap,
    today: String,
    job: F,
) -> Result<Response, AppError>
where
    F: FnOnce(&AppState, &str) -> Result<(Arc<Snapshot>, Bytes), AppError> + Send + 'static,
{
    let current = dated_etag(&state.snapshot(), &today);
    if if_none_match_hits(headers, &current) {
        return Ok(not_modified(current));
    }
    let (etag, body) = reports_api::compute(move || {
        let (snapshot, body) = job(&state, &today)?;
        Ok((dated_etag(&snapshot, &today), body))
    })
    .await?
    .0;
    Ok(ok_json(etag, body))
}

/// The review rows for `indices` (with `fingerprints`, same order), flagged
/// from `entry` (the problem list as of the client's date). An index the
/// snapshot no longer holds is skipped.
fn review_txns<'e>(
    snapshot: &Snapshot,
    entry: &'e DiagnosticsEntry,
    indices: &[Tindex],
    fingerprints: Vec<Option<String>>,
) -> Vec<(usize, ReviewTxn<'e>)> {
    let view = &snapshot.view;
    let journal = &snapshot.journal;
    let declared = view.declared(journal);
    let review = view.review(journal);
    let context = RowContext::new(journal, &declared);
    indices
        .iter()
        .zip(fingerprints)
        .filter_map(|(index, fingerprint)| {
            let at = position_of(snapshot, index.0)?;
            let flags = flags_of(&entry.diagnostics, at);
            let row = context.row(&journal.transactions[at], flags);
            Some((
                at,
                ReviewTxn {
                    row: ReviewRow::at(&review, at, row),
                    fingerprint,
                },
            ))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// GET /api/review/queue
// ---------------------------------------------------------------------------

/// `?kind=unknowns|recents&today&limit&maxDays&offset`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueueQuery {
    kind: Option<String>,
    today: Option<String>,
    limit: Option<usize>,
    max_days: Option<u32>,
    offset: Option<usize>,
}

/// One suggested category (D13a). `pub(crate)`: an import's staged
/// transactions carry the same chips.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireSuggestion {
    account: String,
    /// A rename reviewed transactions with this key received, or null.
    description: Option<String>,
    source: &'static str,
    /// Basis points out of 10000.
    confidence: u32,
    /// Strong enough for the one-keystroke chip (at least 50%).
    strong: bool,
    /// How many transactions back it up.
    count: usize,
    /// A recurring charge's usual amount and funding account, for a
    /// transaction that does not exist yet (D36). Only
    /// `GET /api/review/suggest` fills it in, and it is left off otherwise,
    /// so a queue item's suggestions read exactly as before.
    #[serde(skip_serializing_if = "Option::is_none")]
    recurring: Option<WireRecurringFill>,
}

/// `{amount: {commodity, quantity}, account}`: what an add form fills in
/// from a recurring charge. The amount is the edit wire's own shape, so it
/// goes straight back into a posting.
#[derive(Serialize)]
pub(crate) struct WireRecurringFill {
    amount: WireFillAmount,
    account: String,
}

#[derive(Serialize)]
struct WireFillAmount {
    commodity: String,
    quantity: reports_api::WireDec,
}

impl From<Suggestion> for WireSuggestion {
    fn from(suggestion: Suggestion) -> Self {
        Self {
            strong: suggestion.is_strong(),
            recurring: suggestion.recurring.map(|fill| WireRecurringFill {
                amount: WireFillAmount {
                    commodity: fill.commodity.0,
                    quantity: fill.quantity.into(),
                },
                account: fill.account,
            }),
            source: match suggestion.source {
                SuggestionSource::ExactKey => "exactKey",
                SuggestionSource::RuleFile => "ruleFile",
                SuggestionSource::PayeePrefix => "payeePrefix",
                SuggestionSource::Recurring => "recurring",
            },
            account: suggestion.account,
            description: suggestion.description,
            confidence: suggestion.confidence,
            count: suggestion.count,
        }
    }
}

/// One queued card: its row, fingerprint, similarity key and suggestions.
#[derive(Serialize)]
struct QueueItem<'a> {
    #[serde(flatten)]
    txn: ReviewTxn<'a>,
    /// The normalised payee similar transactions share (D10), or null.
    key: Option<String>,
    suggestions: Vec<WireSuggestion>,
}

/// `{generation, kind, total, offset, notEditable, items}`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct QueueBody<'a> {
    generation: &'a str,
    kind: &'static str,
    total: usize,
    offset: usize,
    not_editable: usize,
    items: Vec<QueueItem<'a>>,
}

/// The queue items for `indices` (with `fingerprints`, same order): each
/// row with its similarity key and suggestions. An index the snapshot no
/// longer holds is skipped.
fn queue_items<'e>(
    snapshot: &Snapshot,
    entry: &'e DiagnosticsEntry,
    indices: &[Tindex],
    fingerprints: Vec<Option<String>>,
) -> Vec<QueueItem<'e>> {
    let journal = &snapshot.journal;
    let (classes, review) = (
        snapshot.view.classes(journal),
        snapshot.view.review(journal),
    );
    review_txns(snapshot, entry, indices, fingerprints)
        .into_iter()
        .map(|(at, txn)| {
            let current = &journal.transactions[at];
            QueueItem {
                txn,
                key: similarity_key(&current.description),
                suggestions: review
                    .suggest(journal, &classes, current.index)
                    .into_iter()
                    .map(WireSuggestion::from)
                    .collect(),
            }
        })
        .collect()
}

fn queue_kind(raw: Option<&str>) -> Result<(QueueKind, &'static str), AppError> {
    match raw {
        Some("unknowns") => Ok((QueueKind::Unknowns, "unknowns")),
        Some("recents") => Ok((QueueKind::Recents, "recents")),
        other => Err(AppError::BadRequest(format!(
            "kind must be unknowns or recents, got {other:?}"
        ))),
    }
}

/// `GET /api/review/queue` — one page of a review queue (D4), newest first.
///
/// `total` is the whole queue's editable count and `notEditable` how many more
/// qualify but live in a file that cannot be written. The queues ignore the
/// Journal filter (Q3). `limit` defaults to 100 for Unknowns and 50 for
/// Recents, and is capped at [`MAX_QUEUE_PAGE`]; `maxDays` defaults to 365,
/// with `0` meaning no limit.
pub(crate) async fn queue(
    State(state): State<AppState>,
    Query(query): Query<QueueQuery>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let (kind, kind_name) = queue_kind(query.kind.as_deref())?;
    let today = parse_date("today", query.today, today_utc)?;
    answer(state, &headers, today, move |state, today| {
        let defaults = QueueOpts::new(kind, today);
        let opts = QueueOpts {
            limit: query.limit.unwrap_or(defaults.limit).min(MAX_QUEUE_PAGE),
            max_days: query.max_days.unwrap_or(defaults.max_days),
            offset: query.offset.unwrap_or(0),
            ..defaults
        };
        let Picked {
            snapshot,
            indices,
            fingerprints,
            found: page,
        } = fingerprinted(state, |snapshot| {
            let page = snapshot
                .view
                .review(&snapshot.journal)
                .queue(&snapshot.journal, &opts);
            Ok((page.items.clone(), page))
        })?;
        let entry = snapshot.view.diagnostics(&snapshot.journal, today);
        let items = queue_items(&snapshot, &entry, &indices, fingerprints);
        let body = to_bytes(&QueueBody {
            generation: generation_of(&snapshot.etag),
            kind: kind_name,
            total: page.total,
            offset: opts.offset,
            not_editable: page.not_editable,
            items,
        })?;
        Ok((Arc::clone(&snapshot), body))
    })
    .await
}

// ---------------------------------------------------------------------------
// GET /api/review/item/{index}
// ---------------------------------------------------------------------------

/// `?today` — the client's date.
#[derive(Deserialize)]
pub(crate) struct ItemQuery {
    today: Option<String>,
}

/// `{generation, item}`.
#[derive(Serialize)]
struct ItemBody<'a> {
    generation: &'a str,
    item: QueueItem<'a>,
}

/// `GET /api/review/item/{index}?today` — one transaction as a queue item
/// (its row, fingerprint, similarity key and suggestions), whether or not it
/// is in a queue: what the card opens on to edit a single transaction (plan
/// 32 D31). A transaction that is not there is a `404`.
pub(crate) async fn item(
    State(state): State<AppState>,
    Path(index): Path<u32>,
    Query(query): Query<ItemQuery>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let today = parse_date("today", query.today, today_utc)?;
    answer(state, &headers, today, move |state, today| {
        let Picked {
            snapshot,
            indices,
            fingerprints,
            ..
        } = fingerprinted(state, |snapshot| {
            if position_of(snapshot, index).is_none() {
                return Err(AppError::NotFound(format!(
                    "no transaction with index {index}"
                )));
            }
            Ok((vec![Tindex(index)], ()))
        })?;
        let entry = snapshot.view.diagnostics(&snapshot.journal, today);
        let item = queue_items(&snapshot, &entry, &indices, fingerprints)
            .into_iter()
            .next()
            .ok_or_else(|| AppError::NotFound(format!("no transaction with index {index}")))?;
        let body = to_bytes(&ItemBody {
            generation: generation_of(&snapshot.etag),
            item,
        })?;
        Ok((Arc::clone(&snapshot), body))
    })
    .await
}

// ---------------------------------------------------------------------------
// GET /api/review/similar/{index}
// ---------------------------------------------------------------------------

/// `?today&queue=1,2,3` — the client's date, and the indices in its queue.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SimilarQuery {
    today: Option<String>,
    queue: Option<String>,
}

/// The most common category among the key's categorised transactions.
#[derive(Serialize)]
struct WirePrecedent {
    account: String,
    count: usize,
    of: usize,
}

/// `{generation, index, key, total, candidates, inQueue, precedent}`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SimilarBody<'a> {
    generation: &'a str,
    index: u32,
    key: Option<String>,
    total: usize,
    candidates: Vec<ReviewTxn<'a>>,
    in_queue: Vec<u32>,
    precedent: Option<WirePrecedent>,
}

/// `?queue=` as a set of indices; absent or empty is the empty set.
fn queue_indices(raw: Option<&str>) -> Result<HashSet<u32>, AppError> {
    let raw = raw.unwrap_or_default();
    let set: HashSet<u32> = raw
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| {
            part.parse::<u32>().map_err(|_| {
                AppError::BadRequest(format!("queue must list transaction indices, got {part:?}"))
            })
        })
        .collect::<Result<_, _>>()?;
    if set.len() > MAX_QUEUE_INDICES {
        return Err(AppError::BadRequest(format!(
            "queue lists {} indices; the limit is {MAX_QUEUE_INDICES}",
            set.len()
        )));
    }
    Ok(set)
}

/// `GET /api/review/similar/{index}` — what D11 offers to apply a card's
/// change to: the unreviewed, editable transactions with the same similarity
/// key (newest first, at most 500 of `total`), the precedent among the
/// categorised ones, and `inQueue` — the candidates that are also in the
/// client's `?queue=`, so applying to them fixes cards still to come.
pub(crate) async fn similar(
    State(state): State<AppState>,
    Path(index): Path<u32>,
    Query(query): Query<SimilarQuery>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let today = parse_date("today", query.today, today_utc)?;
    let queued = queue_indices(query.queue.as_deref())?;
    answer(state, &headers, today, move |state, today| {
        let Picked {
            snapshot,
            indices,
            fingerprints,
            found,
        } = fingerprinted(state, |snapshot| {
            let journal = &snapshot.journal;
            let Some(at) = position_of(snapshot, index) else {
                return Err(AppError::NotFound(format!(
                    "no transaction with index {index}"
                )));
            };
            let found = snapshot.view.review(journal).similar(
                journal,
                &snapshot.view.classes(journal),
                Tindex(index),
            );
            let key = similarity_key(&journal.transactions[at].description);
            Ok((found.candidates.clone(), (found, key)))
        })?;
        let (found, key) = found;
        let in_queue = indices
            .iter()
            .map(|candidate| candidate.0)
            .filter(|candidate| queued.contains(candidate))
            .collect();
        let entry = snapshot.view.diagnostics(&snapshot.journal, today);
        let candidates = review_txns(&snapshot, &entry, &indices, fingerprints)
            .into_iter()
            .map(|(_, txn)| txn)
            .collect();
        let body = to_bytes(&SimilarBody {
            generation: generation_of(&snapshot.etag),
            index,
            key,
            total: found.total,
            candidates,
            in_queue,
            precedent: found.precedent.map(|precedent| WirePrecedent {
                account: precedent.account,
                count: precedent.count,
                of: precedent.of,
            }),
        })?;
        Ok((Arc::clone(&snapshot), body))
    })
    .await
}

// ---------------------------------------------------------------------------
// GET /api/review/similar-key
// ---------------------------------------------------------------------------

/// The longest `?key=` the similar-by-key route reads. A similarity key is a
/// few words; this only bounds a hostile URL.
const MAX_KEY_BYTES: usize = 1_024;

/// `?key&today` — a similarity key (D10) and the client's date.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SimilarKeyQuery {
    key: Option<String>,
    today: Option<String>,
}

/// `GET /api/review/similar-key?key&today` — D24's "already in the journal"
/// group for a card that is not in the journal: a STAGED import transaction's.
/// The body of [`similar`] for a journal transaction with `key`, with
/// `index: 0` (there is none) and `inQueue: []` (an import's run queues no
/// journal transaction). The key is normalised again, so any description's
/// key — or the description itself — reads the same.
pub(crate) async fn similar_key(
    State(state): State<AppState>,
    Query(query): Query<SimilarKeyQuery>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let today = parse_date("today", query.today, today_utc)?;
    let raw = query.key.unwrap_or_default();
    if raw.len() > MAX_KEY_BYTES {
        return Err(AppError::BadRequest(format!(
            "key may be at most {MAX_KEY_BYTES} bytes, got {}",
            raw.len()
        )));
    }
    let key = similarity_key(&raw);
    answer(state, &headers, today, move |state, today| {
        let Picked {
            snapshot,
            indices,
            fingerprints,
            found,
        } = fingerprinted(state, |snapshot| {
            let journal = &snapshot.journal;
            let found = key.as_deref().map_or_else(review::Similar::default, |key| {
                snapshot.view.review(journal).similar_by_key(
                    journal,
                    &snapshot.view.classes(journal),
                    key,
                )
            });
            Ok((found.candidates.clone(), found))
        })?;
        let entry = snapshot.view.diagnostics(&snapshot.journal, today);
        let candidates = review_txns(&snapshot, &entry, &indices, fingerprints)
            .into_iter()
            .map(|(_, txn)| txn)
            .collect();
        let body = to_bytes(&SimilarBody {
            generation: generation_of(&snapshot.etag),
            index: 0,
            key: key.clone(),
            total: found.total,
            candidates,
            in_queue: Vec::new(),
            precedent: found.precedent.map(|precedent| WirePrecedent {
                account: precedent.account,
                count: precedent.count,
                of: precedent.of,
            }),
        })?;
        Ok((Arc::clone(&snapshot), body))
    })
    .await
}

// ---------------------------------------------------------------------------
// GET /api/review/suggest
// ---------------------------------------------------------------------------

/// `?description&today` — the description typed so far, and the client's
/// date.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SuggestQuery {
    description: Option<String>,
    today: Option<String>,
}

/// `{generation, key, suggestions}`.
#[derive(Serialize)]
struct SuggestBody<'a> {
    generation: &'a str,
    /// The description's similarity key (D10), or null when it has none —
    /// and then no suggestions.
    key: Option<String>,
    suggestions: Vec<WireSuggestion>,
}

/// `GET /api/review/suggest?description&today` — D36's "payee picked →
/// category": the suggestions a queue item would carry, for a transaction
/// that does not exist yet ([`ReviewIndex::suggest_for_description`]). A
/// suggestion backed by a recurring charge also carries `recurring`, its
/// usual amount and funding account.
///
/// Descriptions over 1 KB are a `400`.
pub(crate) async fn suggest(
    State(state): State<AppState>,
    Query(query): Query<SuggestQuery>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let today = parse_date("today", query.today, today_utc)?;
    let description = query.description.unwrap_or_default();
    if description.len() > MAX_KEY_BYTES {
        return Err(AppError::BadRequest(format!(
            "description may be at most {MAX_KEY_BYTES} bytes, got {}",
            description.len()
        )));
    }
    answer(state, &headers, today, move |state, _| {
        let snapshot = state.snapshot();
        let journal = &snapshot.journal;
        let suggestions = snapshot
            .view
            .review(journal)
            .suggest_for_description(journal, &snapshot.view.classes(journal), &description)
            .into_iter()
            .map(WireSuggestion::from)
            .collect();
        let body = to_bytes(&SuggestBody {
            generation: generation_of(&snapshot.etag),
            key: similarity_key(&description),
            suggestions,
        })?;
        Ok((snapshot, body))
    })
    .await
}

// ---------------------------------------------------------------------------
// GET /api/payees, GET /api/tags
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct WirePayee<'a> {
    payee: &'a str,
    count: usize,
    last: &'a str,
}

#[derive(Serialize)]
struct WireTagValue<'a> {
    value: &'a str,
    count: usize,
}

#[derive(Serialize)]
struct WireTag<'a> {
    name: &'a str,
    count: usize,
    values: Vec<WireTagValue<'a>>,
}

#[derive(Serialize)]
struct PayeesBody<'a> {
    generation: &'a str,
    payees: Vec<WirePayee<'a>>,
}

#[derive(Serialize)]
struct TagsBody<'a> {
    generation: &'a str,
    tags: Vec<WireTag<'a>>,
}

/// `GET /api/payees` — every payee (the description before `|`) with its use
/// count and newest date, most used first: the description field's
/// completions (D15). Built once per snapshot.
pub(crate) async fn payees(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    conditional(state.snapshot(), &headers, |snapshot| {
        cached(&snapshot.view.payees, || {
            let payees = review::payees(&snapshot.journal);
            to_bytes(&PayeesBody {
                generation: generation_of(&snapshot.etag),
                payees: payees
                    .iter()
                    .map(|use_| WirePayee {
                        payee: &use_.payee,
                        count: use_.count,
                        last: &use_.last,
                    })
                    .collect(),
            })
        })
    })
    .await
}

/// `GET /api/tags` — every transaction tag name with its use count and its
/// commonest values: the tag chip input's completions (D15). Built once per
/// snapshot.
pub(crate) async fn tags(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    conditional(state.snapshot(), &headers, |snapshot| {
        cached(&snapshot.view.tags, || {
            let tags = review::tag_uses(&snapshot.journal);
            to_bytes(&TagsBody {
                generation: generation_of(&snapshot.etag),
                tags: tags
                    .iter()
                    .map(|tag| WireTag {
                        name: &tag.name,
                        count: tag.count,
                        values: tag
                            .values
                            .iter()
                            .map(|(value, count)| WireTagValue {
                                value,
                                count: *count,
                            })
                            .collect(),
                    })
                    .collect(),
            })
        })
    })
    .await
}

/// `slot`'s body, building it with `build` the first time.
fn cached(
    slot: &std::sync::OnceLock<Bytes>,
    build: impl FnOnce() -> Result<Bytes, AppError>,
) -> Result<Bytes, AppError> {
    if let Some(body) = slot.get() {
        return Ok(body.clone());
    }
    let body = build()?;
    Ok(slot.get_or_init(|| body).clone())
}

// ---------------------------------------------------------------------------
// POST /api/transactions/batch
// ---------------------------------------------------------------------------

/// One op on the wire, tagged by `op` (camelCase). See [`EditOp`].
///
/// ```json
/// {"op": "setDescription", "description": "Blue Bottle Coffee"}
/// {"op": "setPostingAccount", "posting": 0, "account": "expenses:food:coffee"}
/// {"op": "setTxnTag", "name": "reviewed", "value": "2026-10-30"}
/// {"op": "setTxnTag", "name": "reviewed", "value": null}
/// {"op": "setTxnComment", "comment": "birthday\nsplit with Sam"}
/// {"op": "setDate", "date": "2026-10-12"}
/// {"op": "setDate2", "date2": "2026-10-14"}
/// {"op": "setDate2", "date2": null}
/// {"op": "setStatus", "status": "cleared"}
/// {"op": "setCode", "code": "1042"}
/// {"op": "setCode", "code": null}
/// {"op": "setPostingComment", "posting": 1, "comment": "gift card\nask Sam"}
/// {"op": "setPostingComment", "posting": 1, "comment": null}
/// {"op": "removePostingAssertion", "posting": 1}
/// {"op": "setPostingTag", "posting": 1, "name": "match", "value": "k7f3a2"}
/// {"op": "setPostingTag", "posting": 1, "name": "match", "value": null}
/// {"op": "addPostingTagValue", "posting": 1, "name": "nomatch", "value": "r4x9q1"}
/// {"op": "removePostingTagValue", "posting": 1, "name": "nomatch", "value": "r4x9q1"}
/// {"op": "replace", "transaction": { …the PUT /api/transactions/{index} body… }}
/// ```
///
/// A `null` (or absent) `date2`, `code` or posting `comment` removes it, and
/// a `null` (or absent) `setPostingTag` value removes every occurrence of the
/// tag on that posting. The posting-tag ops (plan 35) touch only the tag they
/// name, on the posting's own comment; `addPostingTagValue` and
/// `removePostingTagValue` keep a tag's other values (`nomatch:a, nomatch:b`).
/// `setTxnComment` and `setPostingComment` take several lines, joined by
/// `\n`.
#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub(crate) enum EditOpIn {
    SetDescription {
        description: String,
    },
    SetPostingAccount {
        posting: usize,
        account: String,
    },
    SetTxnTag {
        name: String,
        #[serde(default)]
        value: Option<String>,
    },
    SetTxnComment {
        comment: String,
    },
    SetDate {
        date: String,
    },
    SetDate2 {
        #[serde(default)]
        date2: Option<String>,
    },
    SetStatus {
        status: StatusIn,
    },
    SetCode {
        #[serde(default)]
        code: Option<String>,
    },
    SetPostingComment {
        posting: usize,
        #[serde(default)]
        comment: Option<String>,
    },
    RemovePostingAssertion {
        posting: usize,
    },
    SetPostingTag {
        posting: usize,
        name: String,
        #[serde(default)]
        value: Option<String>,
    },
    AddPostingTagValue {
        posting: usize,
        name: String,
        value: String,
    },
    RemovePostingTagValue {
        posting: usize,
        name: String,
        value: String,
    },
    Replace {
        transaction: AddRequest,
    },
}

/// One transaction's edits: the fingerprint it was shown with, and its ops in
/// order.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BatchEditIn {
    index: u32,
    fingerprint: String,
    ops: Vec<EditOpIn>,
}

impl BatchEditIn {
    /// The transaction this edit names.
    pub(crate) fn index(&self) -> Tindex {
        Tindex(self.index)
    }
}

/// `{edits: [{index, fingerprint, ops}]}`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BatchRequest {
    edits: Vec<BatchEditIn>,
}

/// `{generation, edited}`: the new snapshot's generation and every edited
/// transaction as it now reads, in request order.
#[derive(Serialize)]
struct BatchBody<'a> {
    generation: &'a str,
    edited: Vec<ReviewTxn<'a>>,
}

/// The `409` for a transaction whose text changed since the client read it.
pub(crate) fn stale(index: u32) -> AppError {
    AppError::Conflict(format!(
        "transaction #{index} changed since it was loaded (another tab or an external edit); \
         nothing was saved — reload it and try again"
    ))
}

/// The ops' free text checked the way every other write path checks it: no
/// control characters that could end a journal line, and a sane length. The
/// engine refuses what hledger could not read back (a comma in a tag value);
/// this refuses what should never reach it.
fn check_ops(edit: &BatchEditIn) -> Result<(), AppError> {
    check_op_list(&edit.ops)
}

/// [`check_ops`] over a bare op list — what an import's override carries
/// (plan 32 D23): the same ops, aimed at a staged transaction rather than a
/// journal one.
pub(crate) fn check_op_list(ops: &[EditOpIn]) -> Result<(), AppError> {
    for op in ops {
        match op {
            EditOpIn::SetDescription { description } => {
                check_text("a description", description, MAX_DESCRIPTION_BYTES, TAB)?;
            }
            EditOpIn::SetPostingAccount { account, .. } => {
                check_text("an account name", account, MAX_ACCOUNT_BYTES, NONE)?;
            }
            EditOpIn::SetTxnTag { name, value } | EditOpIn::SetPostingTag { name, value, .. } => {
                check_tag(name, value.as_deref())?;
            }
            EditOpIn::AddPostingTagValue { name, value, .. }
            | EditOpIn::RemovePostingTagValue { name, value, .. } => {
                check_tag(name, Some(value))?;
            }
            EditOpIn::SetTxnComment { comment } => {
                check_text("a comment", comment, MAX_COMMENT_BYTES, COMMENT)?;
            }
            EditOpIn::SetDate { date } => check_text("a date", date, MAX_DATE_BYTES, NONE)?,
            EditOpIn::SetDate2 { date2 } => {
                if let Some(date2) = date2 {
                    check_text("a secondary date", date2, MAX_DATE_BYTES, NONE)?;
                }
            }
            EditOpIn::SetCode { code } => {
                if let Some(code) = code {
                    check_text("a code", code, MAX_CODE_BYTES, TAB)?;
                }
            }
            EditOpIn::SetPostingComment { comment, .. } => {
                if let Some(comment) = comment {
                    check_text("a comment", comment, MAX_COMMENT_BYTES, COMMENT)?;
                }
            }
            EditOpIn::SetStatus { .. } | EditOpIn::RemovePostingAssertion { .. } => {}
            EditOpIn::Replace { transaction } => refuse_generated(transaction)?,
        }
    }
    Ok(())
}

/// A tag name and value checked as [`check_op_list`] checks them.
fn check_tag(name: &str, value: Option<&str>) -> Result<(), AppError> {
    check_text("a tag name", name, MAX_TAG_NAME_BYTES, NONE)?;
    match value {
        Some(value) => check_text("a tag value", value, MAX_COMMENT_BYTES, NONE),
        None => Ok(()),
    }
}

/// The engine's ops for one wire edit. A `replace` body is built against the
/// editor's journal, so its amounts take the journal's own styles.
pub(crate) fn engine_ops(journal: &Journal, ops: &[EditOpIn]) -> Result<Vec<EditOp>, AppError> {
    ops.iter()
        .map(|op| {
            Ok(match op {
                EditOpIn::SetDescription { description } => {
                    EditOp::SetDescription(description.clone())
                }
                EditOpIn::SetPostingAccount { posting, account } => EditOp::SetPostingAccount {
                    posting: *posting,
                    account: account.clone(),
                },
                EditOpIn::SetTxnTag { name, value } => EditOp::SetTxnTag {
                    name: name.clone(),
                    value: value.clone(),
                },
                EditOpIn::SetTxnComment { comment } => EditOp::SetTxnComment(comment.clone()),
                EditOpIn::SetDate { date } => EditOp::SetDate(date.clone()),
                EditOpIn::SetDate2 { date2 } => EditOp::SetDate2(date2.clone()),
                EditOpIn::SetStatus { status } => EditOp::SetStatus((*status).into()),
                EditOpIn::SetCode { code } => EditOp::SetCode(code.clone()),
                EditOpIn::SetPostingComment { posting, comment } => EditOp::SetPostingComment {
                    posting: *posting,
                    comment: comment.clone(),
                },
                EditOpIn::RemovePostingAssertion { posting } => {
                    EditOp::RemovePostingAssertion { posting: *posting }
                }
                EditOpIn::SetPostingTag {
                    posting,
                    name,
                    value,
                } => EditOp::SetPostingTag {
                    posting: *posting,
                    name: name.clone(),
                    value: value.clone(),
                },
                EditOpIn::AddPostingTagValue {
                    posting,
                    name,
                    value,
                } => EditOp::AddPostingTagValue {
                    posting: *posting,
                    name: name.clone(),
                    value: value.clone(),
                },
                EditOpIn::RemovePostingTagValue {
                    posting,
                    name,
                    value,
                } => EditOp::RemovePostingTagValue {
                    posting: *posting,
                    name: name.clone(),
                    value: value.clone(),
                },
                EditOpIn::Replace { transaction } => {
                    EditOp::Replace(Box::new(build_transaction(journal, transaction)?))
                }
            })
        })
        .collect()
}

/// An [`EditOp`] failure as HTTP, naming the transaction it belongs to: a
/// stale fingerprint is the `409` [`stale`] words, anything else keeps its
/// usual status with `transaction #N: ` in front.
fn attributed(error: EditError, edits: &[BatchEditIn]) -> AppError {
    match error {
        EditError::BulkTransaction { index, source } => {
            let Some(edit) = edits.get(index) else {
                return AppError::from(*source);
            };
            match *source {
                EditError::ExternalChange => stale(edit.index),
                source => {
                    let message = format!("transaction #{}: {source}", edit.index);
                    AppError::from(source).with_message(message)
                }
            }
        }
        other => AppError::from(other),
    }
}

/// `POST /api/transactions/batch` — apply every edit as one (D14): one lock,
/// one re-parse, one save, all or nothing.
///
/// - `200` `{generation, edited}`: each edited transaction re-read from the
///   new snapshot, with its new fingerprint and markers — what lets the
///   client update its queue at once (D5a). Their `flags` are empty: the
///   problem list of the new snapshot is computed in the background, and the
///   client's revalidation brings it.
/// - `409` naming the first transaction whose fingerprint is stale (or, from
///   the save itself, the file changed on disk).
/// - `404` for an index the journal does not hold; `400` for a malformed body,
///   an op the engine cannot take (`Unsupported`), an imbalance or a text
///   check; `501` when editing is disabled.
pub(crate) async fn batch(
    State(state): State<AppState>,
    payload: Result<Json<BatchRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    let request = json_body(payload)?;
    let body = redacted(&state, batch_locked(&state, &request))?;
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )
        .body(Body::from(body))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()))
}

fn batch_locked(state: &AppState, request: &BatchRequest) -> Result<Bytes, AppError> {
    let edits = &request.edits;
    if edits.is_empty() {
        return Err(AppError::BadRequest(
            "a batch needs at least one edit".to_string(),
        ));
    }
    check_edits(edits)?;

    let mut guard = lock_editor(state)?;
    let indices = apply_edits(bound(&mut guard)?, edits)?;
    save_and_publish(state, &mut guard)?;

    // An edit in place adds, removes and reorders nothing, so every index
    // still names its transaction. Read back under the lock: the snapshot
    // just published and the editor's text agree.
    let fingerprints: Vec<Option<String>> = bound(&mut guard)?
        .transaction_fingerprints(&indices)
        .iter()
        .map(|fingerprint| fingerprint.as_ref().map(Fingerprint::token))
        .collect();
    let snapshot = state.snapshot();
    drop(guard);
    edited_body(&snapshot, &indices, fingerprints)
}

/// The checks a batch's edits pass before the editor is locked: at most
/// [`MAX_BATCH`] of them, and every op's free text sane. Shared with an
/// import's `journalEdits` (plan 32 D24), which are the same edits riding on
/// a commit; an empty list is fine there.
pub(crate) fn check_edits(edits: &[BatchEditIn]) -> Result<(), AppError> {
    if edits.len() > MAX_BATCH {
        return Err(AppError::BadRequest(format!(
            "a batch may edit at most {MAX_BATCH} transactions, got {}",
            edits.len()
        )));
    }
    edits.iter().try_for_each(check_ops)
}

/// `edit`'s fingerprint against the transaction's `current` one: a `404` when
/// there is no such transaction, the `409` [`stale`] words when it changed.
pub(crate) fn check_fingerprint(
    edit: &BatchEditIn,
    current: Option<&Fingerprint>,
) -> Result<(), AppError> {
    match current {
        None => Err(AppError::NotFound(format!(
            "no transaction with index {}",
            edit.index
        ))),
        Some(current) if current.token() != edit.fingerprint => Err(stale(edit.index)),
        Some(_) => Ok(()),
    }
}

/// Check every edit's fingerprint against `editor` and apply them all as ONE
/// [`JournalEditor::apply_batch`] — in memory; the caller saves. Returns the
/// edited transactions' indices, in request order.
///
/// On any error `editor` is unchanged (`apply_batch` is atomic), so there is
/// nothing to re-sync. `pub(crate)`: an import's commit applies its
/// `journalEdits` through this, under the same lock as its own write.
pub(crate) fn apply_edits(
    editor: &mut ledgeline_core::JournalEditor,
    edits: &[BatchEditIn],
) -> Result<Vec<Tindex>, AppError> {
    let indices: Vec<Tindex> = edits.iter().map(BatchEditIn::index).collect();
    // Checked here, by token, so a stale one is named before any op is built;
    // the engine checks the same fingerprints again inside `apply_batch`.
    let current = editor.transaction_fingerprints(&indices);
    let mut batch = Vec::with_capacity(edits.len());
    for (edit, fingerprint) in edits.iter().zip(current) {
        check_fingerprint(edit, fingerprint.as_ref())?;
        let Some(fingerprint) = fingerprint else {
            continue;
        };
        batch.push((
            Tindex(edit.index),
            fingerprint,
            engine_ops(editor.journal(), &edit.ops)?,
        ));
    }
    editor
        .apply_batch(&batch)
        .map_err(|error| attributed(error, edits))?;
    Ok(indices)
}

/// The batch's `200` body over the new `snapshot`. The markers are judged
/// directly ([`is_unknown`], [`ReviewState::of`]) rather than through the new
/// snapshot's review index, which nothing has needed yet and the batch should
/// not wait for.
fn edited_body(
    snapshot: &Snapshot,
    indices: &[Tindex],
    fingerprints: Vec<Option<String>>,
) -> Result<Bytes, AppError> {
    to_bytes(&BatchBody {
        generation: generation_of(&snapshot.etag),
        edited: edited_txns(snapshot, indices, fingerprints),
    })
}

/// Each of `indices` (with `fingerprints`, same order) as a write's response
/// reads it back from the new `snapshot` — see [`edited_body`]. An index the
/// snapshot no longer holds is skipped.
pub(crate) fn edited_txns<'s>(
    snapshot: &'s Snapshot,
    indices: &[Tindex],
    fingerprints: Vec<Option<String>>,
) -> Vec<ReviewTxn<'s>> {
    let journal = &snapshot.journal;
    let classes = snapshot.view.classes(journal);
    let declared = snapshot.view.declared(journal);
    let context = RowContext::new(journal, &declared);
    indices
        .iter()
        .zip(fingerprints)
        .filter_map(|(index, fingerprint)| {
            let txn = &journal.transactions[position_of(snapshot, index.0)?];
            let state = ReviewState::of(txn);
            Some(ReviewTxn {
                row: ReviewRow::marked(
                    context.row(txn, Vec::new()),
                    Some(&state),
                    is_unknown(txn, &classes),
                ),
                fingerprint,
            })
        })
        .collect()
}
