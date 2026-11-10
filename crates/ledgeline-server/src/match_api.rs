//! The transfer-matching routes (plan 35 Phase 2): the proposed pairs, and
//! the writes that link, reject, unlink, forget a rejection or ignore a
//! posting. The model lives in [`ledgeline_core::matching`]; this module is
//! routing, fingerprints and transport, like [`crate::review_api`].
//!
//! # Routes
//!
//! - `GET /api/matches?today&window&lookback&since` —
//!   `{generation, clearing, counts: {clear, check}, older, proposals}`; a
//!   proposal with a non-empty `group` is a suggested combination (A4).
//! - `GET /api/matches/for/{index}?today&window` — the matcher for one
//!   transaction (A2): `{generation, clearing, transaction, postings,
//!   links}`.
//! - `POST /api/matches/link` — `{a, b}` or `{members: [half…]}` (2–16, A3)
//!   → `match:ID` on every posting, and every rejection any carries cleaned
//!   up (D12 rule 3).
//! - `POST /api/matches/reject` — `{a, b}` → a shared `nomatch:ID` (D12).
//! - `POST /api/matches/unlink` — `{matchId, halves}` → `match:ID` removed
//!   wherever it is (D5).
//! - `POST /api/matches/forget` — `{value, halves}` → `nomatch:VALUE`
//!   removed wherever it is (D12 Undo).
//! - `POST /api/matches/ignore` — `{half}` → `match:ignore` (D1).
//! - `POST /api/matches/link-all` — `{pairs: [{a, b}]}`: "Link all clear
//!   matches" (D9), written in [`MAX_BATCH`]-sized batches.
//!
//! # Halves
//!
//! Every write names its postings as `{index, posting, fingerprint,
//! description?}`: the transaction's `tindex`, the posting's position among
//! the postings the FILE writes (as `setPostingAccount` counts them), the
//! fingerprint the transaction was served with, and optionally a new
//! description for that transaction, written in the SAME batch (D9: "Link and
//! Save"). A proposal's halves carry exactly these fields.
//!
//! # One write each
//!
//! The server builds every op itself, through [`matching`], so a client
//! cannot skip the D12 cleanup or write a link that does not cancel. Each
//! write is one lock, one [`JournalEditor::apply_batch`], one save — the
//! batch endpoint's discipline. A named transaction whose text changed since
//! it was served is a `409` and nothing is written. Transactions the server
//! adds on its own (the other side of a rejection being cleaned up, the third
//! posting of a broken link) are taken at their current text, which the ops
//! were built from under the same lock.
//!
//! Responses are `{generation, id, edited}`: the new snapshot, the id written
//! (`match`/`nomatch` value, `"ignore"`, or the one removed), and every
//! transaction the write touched, in file order, as the batch serves them.

use crate::edit_api::{
    MAX_DESCRIPTION_BYTES, TAB, bound, check_text, json_body, lock_editor, redacted,
    resync_from_disk, save_and_publish,
};
use crate::error::AppError;
use crate::journal_api::{flags_of, position_of, to_bytes};
use crate::reports_api::{parse_date, today_utc};
use crate::review_api::{
    MAX_BATCH, Picked, ReviewRow, ReviewTxn, answer, edited_txns, fingerprinted, stale,
};
use crate::{AppState, Snapshot, generation_of};
use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use ledgeline_core::matching::{
    self, IGNORE_VALUE, Linker, MATCH_TAG, MAX_LINK_MEMBERS, MatchError, MatchIds, MatchSettings,
    NOMATCH_TAG, PostingRef, Proposal, TxnOps,
};
use ledgeline_core::model::{Journal, Tindex};
use ledgeline_core::view::rows::RowContext;
use ledgeline_core::{EditError, EditOp, Fingerprint, JournalEditor};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

/// The match window's range in days (D8).
const WINDOW_DAYS: std::ops::RangeInclusive<u32> = 1..=60;
/// The longest lookback in days (D8); `0` means no limit.
const MAX_LOOKBACK_DAYS: u32 = 36_500;
/// The most pairs one "Link all" may name. Far more than a first run on a
/// decades-old journal proposes; it only bounds a hostile body.
const MAX_BULK_PAIRS: usize = 100_000;
/// The most halves an unlink or forget may name.
const MAX_HALVES: usize = 16;

// ---------------------------------------------------------------------------
// GET /api/matches
// ---------------------------------------------------------------------------

/// `?today&window&lookback&since`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MatchesQuery {
    today: Option<String>,
    window: Option<u32>,
    lookback: Option<u32>,
    since: Option<String>,
}

/// One half of a pair: the transaction as a review route serves it (row,
/// markers, fingerprint), plus which written posting is its clearing leg.
#[derive(Serialize)]
struct WireHalf<'a> {
    #[serde(flatten)]
    txn: ReviewTxn<'a>,
    posting: usize,
    /// The name of the file the transaction is written in, without its
    /// directory (the card's last line, D9); empty for a journal read from
    /// no file.
    file: &'a str,
}

/// Another legal pair sharing a half with a proposal (the card's radio
/// choices, D9).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireAlternative<'a> {
    a: WireHalf<'a>,
    b: WireHalf<'a>,
    gap_days: u32,
    score: i32,
    /// The half it does not share with the proposal is in another proposal.
    taken: bool,
}

/// One end of a proposal's "Checking → Amex": the transaction, its real
/// account and short label — `journal_api::WireTransferEnd`'s shape, with the
/// label owned (it is computed per proposal).
#[derive(Serialize)]
struct WireEnd<'a> {
    index: u32,
    account: Option<&'a str>,
    label: Option<String>,
}

/// One proposed pair, or a suggested combination (`group`).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireProposal<'a> {
    /// The earlier half (by effective date, then file order).
    a: WireHalf<'a>,
    b: WireHalf<'a>,
    gap_days: u32,
    score: i32,
    /// Unambiguous with a non-negative score: in "Link all clear".
    clear: bool,
    ambiguous: bool,
    /// The card's header, "Checking → Amex": the source end (its real
    /// posting is negative) and the destination.
    from: WireEnd<'a>,
    to: WireEnd<'a>,
    alternatives: Vec<WireAlternative<'a>>,
    /// Empty for a pair. For a suggested combination (A4): the many side,
    /// 2–4 halves summing to exactly `a`'s negation, nearest first; `a` is
    /// the single posting and `b` is `group[0]`. Never `clear`.
    group: Vec<WireHalf<'a>>,
}

#[derive(Serialize)]
struct WireCounts {
    clear: usize,
    check: usize,
}

/// `{generation, clearing, counts, proposals}`.
#[derive(Serialize)]
struct MatchesBody<'a> {
    generation: &'a str,
    /// Whether any account directive declares `clearing:` (D2). With none,
    /// Find Matches is disabled and there are never any proposals.
    clearing: bool,
    counts: WireCounts,
    /// How many more proposals there would be with no lookback (A1): the
    /// matcher's count with `lookback=0` less the proposals here, never
    /// below 0; `0` when the lookback is already 0.
    older: usize,
    proposals: Vec<WireProposal<'a>>,
}

/// `?window` and `?lookback` checked against D8's ranges, with D8's defaults.
fn match_settings(window: Option<u32>, lookback: Option<u32>) -> Result<MatchSettings, AppError> {
    let defaults = MatchSettings::default();
    let window_days = window.unwrap_or(defaults.window_days);
    if !WINDOW_DAYS.contains(&window_days) {
        return Err(AppError::BadRequest(format!(
            "window must be {} to {} days, got {window_days}",
            WINDOW_DAYS.start(),
            WINDOW_DAYS.end()
        )));
    }
    let lookback_days = lookback.unwrap_or(defaults.lookback_days);
    if lookback_days > MAX_LOOKBACK_DAYS {
        return Err(AppError::BadRequest(format!(
            "lookback must be 0 (no limit) to {MAX_LOOKBACK_DAYS} days, got {lookback_days}"
        )));
    }
    Ok(MatchSettings {
        window_days,
        lookback_days,
        involving: None,
    })
}

/// `?since=JOURNAL_ID:COUNT` — the import entry point's scope (D9): the last
/// `COUNT` transactions of the journal file `JOURNAL_ID`, which is exactly
/// what an import commit appended (`journalWritten` and `imported` in its
/// response). A journal id never contains `:`, so the last one splits it.
///
/// The id must name a file the journal reads; a count past the file's
/// transactions takes all of them.
fn since_scope(snapshot: &Snapshot, since: &str) -> Result<HashSet<Tindex>, AppError> {
    let bad = |why: &str| AppError::BadRequest(format!("since must be JOURNAL_ID:COUNT; {why}"));
    let (id, count) = since
        .rsplit_once(':')
        .ok_or_else(|| bad("there is no `:`"))?;
    let count: usize = count
        .parse()
        .map_err(|_| bad("the count is not a number"))?;
    let journal = &snapshot.raw;
    let file = snapshot
        .location
        .as_deref()
        .and_then(|location| {
            let path = location.source_path(journal, id)?;
            Some(location.key(&path))
        })
        .ok_or_else(|| bad("the id names no file this journal reads"))?;
    Ok(journal
        .transactions
        .iter()
        .rev()
        .filter(|txn| txn.source_file == file)
        .take(count)
        .map(|txn| txn.index)
        .collect())
}

/// `GET /api/matches?today&window&lookback&since` — the proposed transfer
/// pairs (D4, D6, D7), earliest first, each half a full review row with its
/// fingerprint and posting position, ready to link.
///
/// `window` (default 10, 1–60) and `lookback` (default 365, `0` = no limit)
/// are D8's settings; `since` scopes to pairs with at least one half among an
/// import's new transactions (see [`since_scope`]) — assignment still runs
/// over every candidate first. `counts` are over the proposals returned.
pub(crate) async fn proposals(
    State(state): State<AppState>,
    Query(query): Query<MatchesQuery>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let today = parse_date("today", query.today, today_utc)?;
    let settings = match_settings(query.window, query.lookback)?;
    let since = query.since;
    answer(state, &headers, today, move |state, today| {
        let Picked {
            snapshot,
            indices,
            fingerprints,
            found: proposals,
        } = fingerprinted(state, |snapshot| {
            let settings = MatchSettings {
                involving: since
                    .as_deref()
                    .map(|since| since_scope(snapshot, since))
                    .transpose()?,
                ..settings
            };
            // The raw journal: a proposal's posting positions must be the
            // ones the file writes, which the write routes address.
            let proposals = matching::propose(&snapshot.raw, &settings, today);
            // A second run with no lookback, counted only (A1). Not a filter
            // of that run: the assignment is global, so a posting inside the
            // lookback may pair differently once older ones compete for it.
            let older = if settings.lookback_days == 0 {
                0
            } else {
                let unlimited = MatchSettings {
                    lookback_days: 0,
                    ..settings
                };
                matching::count_proposals(&snapshot.raw, &unlimited, today)
                    .saturating_sub(proposals.len())
            };
            let indices = txns_of(proposals.iter().flat_map(proposal_postings));
            Ok((indices, (proposals, older)))
        })?;
        let (proposals, older) = proposals;
        let labels = matching::AccountLabels::of(&snapshot.raw);
        let body = render(
            &snapshot,
            today,
            &indices,
            fingerprints,
            |half| -> Result<Bytes, AppError> {
                let proposals = proposals
                    .iter()
                    .map(|proposal| wire_proposal(&snapshot, &labels, half, proposal))
                    .collect::<Vec<_>>();
                let clear = proposals.iter().filter(|proposal| proposal.clear).count();
                to_bytes(&MatchesBody {
                    generation: generation_of(&snapshot.etag),
                    clearing: matching::has_clearing_accounts(&snapshot.raw),
                    counts: WireCounts {
                        clear,
                        check: proposals.len() - clear,
                    },
                    older,
                    proposals,
                })
            },
        )?;
        Ok((Arc::clone(&snapshot), body))
    })
    .await
}

/// Every posting a proposal's wire names: its halves, its group, and its
/// alternatives' halves.
fn proposal_postings(proposal: &Proposal) -> impl Iterator<Item = PostingRef> + '_ {
    proposal
        .postings()
        .chain(proposal.alternatives.iter().flat_map(|alt| [alt.a, alt.b]))
}

/// The distinct transactions of `postings`, in index order.
fn txns_of(postings: impl Iterator<Item = PostingRef>) -> Vec<Tindex> {
    let mut indices: Vec<Tindex> = postings.map(|at| at.txn).collect();
    indices.sort_unstable_by_key(|index| index.0);
    indices.dedup();
    indices
}

/// Builds each half's wire: the review row, its fingerprint (from
/// `fingerprints`, one per `indices`), the posting and the file.
type HalfOf<'x> = dyn Fn(PostingRef) -> WireHalf<'x> + 'x;

/// Call `body` with a [`HalfOf`] over `snapshot` as of `today`.
fn render<R>(
    snapshot: &Snapshot,
    today: &str,
    indices: &[Tindex],
    fingerprints: Vec<Option<String>>,
    body: impl for<'x> FnOnce(&'x HalfOf<'x>) -> R,
) -> R {
    let journal = &snapshot.journal;
    let view = &snapshot.view;
    let entry = view.diagnostics(journal, today);
    let declared = view.declared(journal);
    let review = view.review(journal);
    let context = RowContext::new(journal, &declared);
    let fingerprint_of: HashMap<Tindex, Option<String>> =
        indices.iter().copied().zip(fingerprints).collect();
    let half = |at: PostingRef| {
        let txn = &journal.transactions[at.position];
        let row = context.row(txn, flags_of(&entry.diagnostics, at.position));
        WireHalf {
            txn: ReviewTxn {
                row: ReviewRow::at(&review, at.position, row),
                fingerprint: fingerprint_of.get(&at.txn).cloned().flatten(),
            },
            posting: at.posting,
            file: txn
                .source_file
                .file_name()
                .and_then(std::ffi::OsStr::to_str)
                .unwrap_or_default(),
        }
    };
    body(&half)
}

/// One proposal on the wire, its halves built by `half`.
fn wire_proposal<'a, 'h: 'a, 'p: 'a>(
    snapshot: &Snapshot,
    labels: &matching::AccountLabels,
    half: &HalfOf<'h>,
    proposal: &'p Proposal,
) -> WireProposal<'a> {
    let [from, to] = matching::direction(&snapshot.raw, proposal.a, proposal.b);
    let accounts = [from, to].map(|at| {
        if at == proposal.a {
            proposal.real_accounts[0].as_deref()
        } else {
            proposal.real_accounts[1].as_deref()
        }
    });
    let [from_label, to_label] = labels.pair(accounts[0], accounts[1]);
    WireProposal {
        a: half(proposal.a),
        b: half(proposal.b),
        gap_days: proposal.gap_days,
        score: proposal.score,
        clear: proposal.clear,
        ambiguous: proposal.ambiguous,
        from: WireEnd {
            index: from.txn.0,
            account: accounts[0],
            label: from_label,
        },
        to: WireEnd {
            index: to.txn.0,
            account: accounts[1],
            label: to_label,
        },
        alternatives: proposal
            .alternatives
            .iter()
            .map(|alt| WireAlternative {
                a: half(alt.a),
                b: half(alt.b),
                gap_days: alt.gap_days,
                score: alt.score,
                taken: alt.taken,
            })
            .collect(),
        group: proposal.group.iter().map(|&at| half(at)).collect(),
    }
}

// ---------------------------------------------------------------------------
// GET /api/matches/for/{index}
// ---------------------------------------------------------------------------

/// `?today&window`.
#[derive(Deserialize)]
pub(crate) struct ForQuery {
    today: Option<String>,
    window: Option<u32>,
}

/// Another posting offered for one of the transaction's: a full half plus
/// its gap in days.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WirePartner<'a> {
    #[serde(flatten)]
    half: WireHalf<'a>,
    gap_days: u32,
}

/// What the matcher offers one clearing posting of the transaction (A2).
#[derive(Serialize)]
struct WirePostingPartners<'a> {
    /// The posting's position among the postings the file writes.
    posting: usize,
    /// Exact opposite amounts at any gap, not rejected with it, nearest
    /// first, at most 20.
    exact: Vec<WirePartner<'a>>,
    /// The suggested combinations (A4) it is in, shaped as `/api/matches`
    /// proposals (`group` non-empty).
    combinations: Vec<WireProposal<'a>>,
    /// Opposite-sign postings on its account within ± the window, nearest
    /// first, at most 50.
    nearby: Vec<WirePartner<'a>>,
}

/// A `match` id on one of the transaction's postings.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireLink<'a> {
    match_id: &'a str,
    /// Not a valid link (D10, A3): Unlink is its fix.
    broken: bool,
    /// Every posting carrying the id, in file order, this transaction's
    /// included.
    members: Vec<WireHalf<'a>>,
}

/// `{generation, clearing, transaction, postings, links}`.
#[derive(Serialize)]
struct ForBody<'a> {
    generation: &'a str,
    clearing: bool,
    /// The transaction asked about, as a review route serves it.
    transaction: ReviewTxn<'a>,
    /// One entry per unlinked, not ignored clearing posting, in posting
    /// order; empty when it has none.
    postings: Vec<WirePostingPartners<'a>>,
    /// The links (valid or broken) its postings carry, in id order.
    links: Vec<WireLink<'a>>,
}

/// The ids of the links touching `position` and their postings.
type TouchingLinks = Vec<(String, bool, Vec<PostingRef>)>;

fn links_touching(journal: &Journal, position: usize) -> TouchingLinks {
    let index = matching::links(journal);
    let mut found: TouchingLinks = index
        .links_at(position)
        .iter()
        .filter_map(|id| Some((id.clone(), false, index.members(id)?)))
        .chain(
            index
                .broken()
                .iter()
                .filter(|link| link.postings.iter().any(|at| at.position == position))
                .map(|link| (link.id.clone(), true, link.postings.clone())),
        )
        .collect();
    found.sort_by(|x, y| x.0.cmp(&y.0));
    found
}

/// `GET /api/matches/for/{index}?today&window` — the matcher for one
/// transaction (A2): for each of its unlinked, not `match:ignore` clearing
/// postings, its exact partners (any gap, no lookback), the suggested
/// combinations it is in, and the opposite postings within ± `window`
/// (default 10, 1–60) — plus the links it already has, so an editor can
/// offer Unlink. `404` when there is no such transaction; empty lists when
/// there is nothing to offer.
pub(crate) async fn for_transaction(
    State(state): State<AppState>,
    Path(index): Path<u32>,
    Query(query): Query<ForQuery>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let today = parse_date("today", query.today, today_utc)?;
    let window_days = match_settings(query.window, None)?.window_days;
    answer(state, &headers, today, move |state, today| {
        let Picked {
            snapshot,
            indices,
            fingerprints,
            found: (at, partners, links),
        } = fingerprinted(state, |snapshot| {
            let position = position_of(snapshot, index)
                .ok_or_else(|| AppError::NotFound(format!("no transaction with index {index}")))?;
            let at = PostingRef {
                position,
                txn: Tindex(index),
                posting: 0,
            };
            let partners = matching::partners_for(&snapshot.raw, position, window_days, today);
            let links = links_touching(&snapshot.raw, position);
            let indices = txns_of(
                std::iter::once(at)
                    .chain(partners.iter().flat_map(|one| {
                        one.exact
                            .iter()
                            .chain(&one.nearby)
                            .map(|partner| partner.at)
                            .chain(one.combinations.iter().flat_map(Proposal::postings))
                    }))
                    .chain(
                        links
                            .iter()
                            .flat_map(|(_, _, members)| members.iter().copied()),
                    ),
            );
            Ok((indices, (at, partners, links)))
        })?;
        let labels = matching::AccountLabels::of(&snapshot.raw);
        let body = render(&snapshot, today, &indices, fingerprints, |half| {
            let partner = |partner: &matching::Partner| WirePartner {
                half: half(partner.at),
                gap_days: partner.gap_days,
            };
            to_bytes(&ForBody {
                generation: generation_of(&snapshot.etag),
                clearing: matching::has_clearing_accounts(&snapshot.raw),
                transaction: half(at).txn,
                postings: partners
                    .iter()
                    .map(|one| WirePostingPartners {
                        posting: one.at.posting,
                        exact: one.exact.iter().map(partner).collect(),
                        combinations: one
                            .combinations
                            .iter()
                            .map(|proposal| wire_proposal(&snapshot, &labels, half, proposal))
                            .collect(),
                        nearby: one.nearby.iter().map(partner).collect(),
                    })
                    .collect(),
                links: links
                    .iter()
                    .map(|(id, broken, members)| WireLink {
                        match_id: id,
                        broken: *broken,
                        members: members.iter().map(|&at| half(at)).collect(),
                    })
                    .collect(),
            })
        })?;
        Ok((Arc::clone(&snapshot), body))
    })
    .await
}

// ---------------------------------------------------------------------------
// The writes
// ---------------------------------------------------------------------------

/// One posting a write names: its transaction's `tindex`, its position among
/// the postings the file writes, the fingerprint the transaction was served
/// with, and optionally a new description for the transaction, written in
/// the same batch.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct HalfIn {
    index: u32,
    posting: usize,
    fingerprint: String,
    #[serde(default)]
    description: Option<String>,
}

/// `{a, b}`: the two halves of a pair.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PairIn {
    a: HalfIn,
    b: HalfIn,
}

/// `POST /api/matches/link`: `{a, b}` (a pair), or `{members}` (a group of
/// 2 to [`MAX_LINK_MEMBERS`] postings, A3) — one or the other.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LinkIn {
    #[serde(default)]
    a: Option<HalfIn>,
    #[serde(default)]
    b: Option<HalfIn>,
    #[serde(default)]
    members: Option<Vec<HalfIn>>,
}

impl LinkIn {
    /// The postings to link, or a `400` for a body that is neither shape.
    fn members(&self) -> Result<Vec<&HalfIn>, AppError> {
        match (&self.a, &self.b, &self.members) {
            (Some(a), Some(b), None) => Ok(vec![a, b]),
            (None, None, Some(members)) if (2..=MAX_LINK_MEMBERS).contains(&members.len()) => {
                Ok(members.iter().collect())
            }
            (None, None, Some(members)) => Err(AppError::BadRequest(format!(
                "link 2 to {MAX_LINK_MEMBERS} members, got {}",
                members.len()
            ))),
            _ => Err(AppError::BadRequest(
                "name either `a` and `b`, or `members`".to_string(),
            )),
        }
    }
}

/// `{matchId, halves}`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UnlinkIn {
    match_id: String,
    halves: Vec<HalfIn>,
}

/// `{value, halves}`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ForgetIn {
    value: String,
    halves: Vec<HalfIn>,
}

/// `{half}`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct IgnoreIn {
    half: HalfIn,
}

/// `{pairs: [{a, b}]}`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LinkAllIn {
    pairs: Vec<PairIn>,
}

/// `{generation, id, edited}`.
#[derive(Serialize)]
struct WriteBody<'a> {
    generation: &'a str,
    id: &'a str,
    edited: Vec<ReviewTxn<'a>>,
}

/// `{generation, linked, matchIds}`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LinkAllBody<'a> {
    generation: &'a str,
    /// How many pairs were linked: all of them.
    linked: usize,
    /// Each pair's new `match` id, in request order.
    match_ids: Vec<String>,
}

fn ok(body: Bytes) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )
        .body(Body::from(body))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// A [`MatchError`] as HTTP: a posting that is not there is a `404`;
/// anything else (not matchable, not a pair, an unknown id) is a `400`. A
/// stale client is caught by its fingerprints before the ops are built.
fn match_error(error: MatchError) -> AppError {
    match error {
        MatchError::PostingNotFound(_) => AppError::NotFound(error.to_string()),
        _ => AppError::BadRequest(error.to_string()),
    }
}

/// `halves` resolved against `journal` and checked against `editor`'s
/// fingerprints: a `404` for a transaction that is not there, the batch's
/// `409` for one whose text changed since it was served, and every
/// description checked as the batch checks one.
fn resolve(
    editor: &JournalEditor,
    journal: &Journal,
    halves: &[&HalfIn],
) -> Result<Vec<PostingRef>, AppError> {
    for half in halves {
        if let Some(description) = &half.description {
            check_text("a description", description, MAX_DESCRIPTION_BYTES, TAB)?;
        }
    }
    let indices: Vec<Tindex> = halves.iter().map(|half| Tindex(half.index)).collect();
    let current = editor.transaction_fingerprints(&indices);
    halves
        .iter()
        .zip(current)
        .map(|(half, current)| {
            let position = position_in(journal, half.index)
                .filter(|_| current.is_some())
                .ok_or_else(|| {
                    AppError::NotFound(format!("no transaction with index {}", half.index))
                })?;
            if current.is_some_and(|current| current.token() != half.fingerprint) {
                return Err(stale(half.index));
            }
            Ok(PostingRef {
                position,
                txn: Tindex(half.index),
                posting: half.posting,
            })
        })
        .collect()
}

/// The position of the transaction with `tindex` in `journal` (see
/// `journal_api::position_of`; this one reads the editor's journal).
fn position_in(journal: &Journal, tindex: u32) -> Option<usize> {
    let txns = &journal.transactions;
    let guess = usize::try_from(tindex).ok()?.checked_sub(1)?;
    if txns.get(guess).is_some_and(|txn| txn.index.0 == tindex) {
        return Some(guess);
    }
    txns.iter().position(|txn| txn.index.0 == tindex)
}

/// Whether the posting `at` carries `name:value`.
fn carries(journal: &Journal, at: PostingRef, name: &str, value: &str) -> bool {
    journal
        .transactions
        .get(at.position)
        .and_then(|txn| txn.postings.get(at.posting))
        .is_some_and(|posting| {
            posting
                .tags
                .iter()
                .any(|(tag, tagged)| tag == name && tagged == value)
        })
}

/// Ops per transaction, merged across actions in file order, with an op
/// already queued for a transaction not queued again (a rejection two linked
/// pairs both clean up is removed once).
#[derive(Default)]
struct Merged(BTreeMap<u32, Vec<EditOp>>);

impl Merged {
    fn add(&mut self, ops: TxnOps) {
        for (index, ops) in ops {
            let queued = self.0.entry(index.0).or_default();
            for op in ops {
                if !queued.contains(&op) {
                    queued.push(op);
                }
            }
        }
    }

    /// Each half's new description, where it changes the transaction's.
    fn describe(&mut self, journal: &Journal, halves: &[(&HalfIn, PostingRef)]) {
        for (half, at) in halves {
            let Some(description) = &half.description else {
                continue;
            };
            if journal.transactions[at.position].description != *description {
                self.add(vec![(
                    at.txn,
                    vec![EditOp::SetDescription(description.clone())],
                )]);
            }
        }
    }

    fn len(&self) -> usize {
        self.0.len()
    }

    /// Apply every op as ONE [`JournalEditor::apply_batch`], each transaction
    /// at its current text (the ops were built from it under this lock).
    /// Returns the edited transactions, in file order.
    fn apply(self, editor: &mut JournalEditor) -> Result<Vec<Tindex>, AppError> {
        let indices: Vec<Tindex> = self.0.keys().copied().map(Tindex).collect();
        let current = editor.transaction_fingerprints(&indices);
        let batch: Vec<(Tindex, Fingerprint, Vec<EditOp>)> = self
            .0
            .into_iter()
            .zip(current)
            .map(|((index, ops), fingerprint)| {
                let fingerprint = fingerprint.ok_or_else(|| {
                    AppError::NotFound(format!("no transaction with index {index}"))
                })?;
                Ok((Tindex(index), fingerprint, ops))
            })
            .collect::<Result<_, AppError>>()?;
        editor.apply_batch(&batch).map_err(|error| match error {
            EditError::BulkTransaction { index, source } => {
                let Some((tindex, _, _)) = batch.get(index) else {
                    return AppError::from(*source);
                };
                match *source {
                    EditError::ExternalChange => stale(tindex.0),
                    source => {
                        let message = format!("transaction #{}: {source}", tindex.0);
                        AppError::from(source).with_message(message)
                    }
                }
            }
            other => AppError::from(other),
        })?;
        Ok(indices)
    }
}

/// Whether `id` is now a valid link of exactly `members` — what a link
/// write must leave behind (D5: a stale client can never write a broken one).
fn linked(journal: &Journal, id: &str, members: &[PostingRef]) -> bool {
    matching::links(journal).members(id).is_some_and(|found| {
        found.len() == members.len() && members.iter().all(|at| found.contains(at))
    })
}

/// What one locked write did: the id it wrote and the transactions it
/// touched.
struct Written {
    id: String,
    indices: Vec<Tindex>,
}

/// Lock the editor, resolve and fingerprint-check `halves`, build the ops
/// with `build` (from the editor's journal, under the lock), add the halves'
/// description edits, apply, `verify` the result, save, and answer with the
/// touched transactions.
fn write_one(
    state: &AppState,
    halves: &[&HalfIn],
    build: impl FnOnce(&Journal, &[PostingRef]) -> Result<(String, TxnOps), AppError>,
    verify: impl FnOnce(&Journal, &str, &[PostingRef]) -> bool,
) -> Result<Bytes, AppError> {
    let mut guard = lock_editor(state)?;
    let editor = bound(&mut guard)?;
    let journal = Arc::clone(editor.journal());
    let refs = resolve(editor, &journal, halves)?;
    let (id, ops) = build(&journal, &refs)?;
    let mut merged = Merged::default();
    merged.add(ops);
    let named: Vec<(&HalfIn, PostingRef)> = halves.iter().copied().zip(refs.clone()).collect();
    merged.describe(&journal, &named);
    let indices = merged.apply(editor)?;
    if !verify(editor.journal(), &id, &refs) {
        // Unreachable while the ops are built from the journal they apply
        // to; if it ever is reached, nothing unsound reaches the file.
        resync_from_disk(state, &mut guard)?;
        return Err(AppError::Internal(
            "the match write did not read back as asked; nothing was saved".to_string(),
        ));
    }
    save_and_publish(state, &mut guard)?;
    written_body(state, &mut guard, &Written { id, indices })
}

/// The `200` body for a write: the touched transactions, re-read under the
/// lock from the snapshot just published (see `review_api::batch`).
fn written_body(
    state: &AppState,
    guard: &mut Option<JournalEditor>,
    written: &Written,
) -> Result<Bytes, AppError> {
    let fingerprints: Vec<Option<String>> = bound(guard)?
        .transaction_fingerprints(&written.indices)
        .iter()
        .map(|fingerprint| fingerprint.as_ref().map(Fingerprint::token))
        .collect();
    let snapshot = state.snapshot();
    to_bytes(&WriteBody {
        generation: generation_of(&snapshot.etag),
        id: &written.id,
        edited: edited_txns(&snapshot, &written.indices, fingerprints),
    })
}

fn pair_halves(pair: &PairIn) -> [&HalfIn; 2] {
    [&pair.a, &pair.b]
}

/// `POST /api/matches/link` — link the pair `{a, b}` (D1), or the group
/// `{members: [half…]}` (2 to 16 postings, A3): a fresh `match` id on every
/// clearing posting, every `nomatch` value any carries removed from both of
/// its postings (D12 rule 3), and each half's `description` if given — one
/// write.
///
/// `400` when the postings are not a legal link (not candidates, two in one
/// transaction, different clearing accounts, amounts that do not sum to
/// exactly zero in one commodity) or the body is neither shape; `404` for a
/// missing transaction or posting; `409` for a stale fingerprint.
pub(crate) async fn link(
    State(state): State<AppState>,
    payload: Result<Json<LinkIn>, JsonRejection>,
) -> Result<Response, AppError> {
    let request = json_body(payload)?;
    let body = redacted(&state, {
        request.members().and_then(|members| {
            write_one(
                &state,
                &members,
                |journal, refs| {
                    let id = matching::new_match_id(journal);
                    let ops = matching::link_group_ops(journal, refs, &id).map_err(match_error)?;
                    Ok((id, ops))
                },
                linked,
            )
        })
    })?;
    Ok(ok(body))
}

/// `POST /api/matches/reject` — "Not a match" (D12): a fresh `nomatch` id on
/// both halves, plus each half's `description` if given — one write. Errors
/// as [`link`].
pub(crate) async fn reject(
    State(state): State<AppState>,
    payload: Result<Json<PairIn>, JsonRejection>,
) -> Result<Response, AppError> {
    let request = json_body(payload)?;
    let body = redacted(
        &state,
        write_one(
            &state,
            &pair_halves(&request),
            |journal, refs| {
                let id = matching::new_match_id(journal);
                let ops =
                    matching::reject_ops(journal, refs[0], refs[1], &id).map_err(match_error)?;
                Ok((id, ops))
            },
            |journal, id, refs| refs.iter().all(|at| carries(journal, *at, NOMATCH_TAG, id)),
        ),
    )?;
    Ok(ok(body))
}

/// The halves an unlink or forget names: at least one, at most
/// [`MAX_HALVES`], each carrying `name:value` — so the client proves it saw
/// the tag it is removing.
fn tagged_halves<'h>(
    halves: &'h [HalfIn],
    name: &'static str,
    value: &str,
) -> Result<Vec<&'h HalfIn>, AppError> {
    if halves.is_empty() || halves.len() > MAX_HALVES {
        return Err(AppError::BadRequest(format!(
            "name 1 to {MAX_HALVES} postings carrying `{name}:{value}`, got {}",
            halves.len()
        )));
    }
    Ok(halves.iter().collect())
}

/// Remove `name:value` everywhere (`ops`), after checking the named halves
/// carry it.
fn removal(
    journal: &Journal,
    refs: &[PostingRef],
    name: &'static str,
    value: &str,
    ops: impl FnOnce(&Journal, &str) -> Result<TxnOps, MatchError>,
) -> Result<(String, TxnOps), AppError> {
    if let Some(at) = refs.iter().find(|at| !carries(journal, **at, name, value)) {
        return Err(AppError::BadRequest(format!(
            "posting #{} of transaction #{} does not carry `{name}:{value}`",
            at.posting, at.txn.0
        )));
    }
    Ok((value.to_string(), ops(journal, value).map_err(match_error)?))
}

/// After a removal, no posting carries `name:value` any more.
fn removed(journal: &Journal, name: &str, value: &str) -> bool {
    journal
        .transactions
        .iter()
        .flat_map(|txn| &txn.postings)
        .all(|posting| {
            !posting
                .tags
                .iter()
                .any(|(tag, tagged)| tag == name && tagged == value)
        })
}

/// `POST /api/matches/unlink` — `{matchId, halves}`: remove `match:ID` from
/// every posting carrying it (D5), a broken link's third posting included;
/// the files then read as they did before the link. Each named half must
/// carry it (`400` otherwise) and be fresh (`409`).
pub(crate) async fn unlink(
    State(state): State<AppState>,
    payload: Result<Json<UnlinkIn>, JsonRejection>,
) -> Result<Response, AppError> {
    let request = json_body(payload)?;
    let id = request.match_id.clone();
    let body = redacted(&state, {
        tagged_halves(&request.halves, MATCH_TAG, &id).and_then(|halves| {
            if id == IGNORE_VALUE {
                return Err(AppError::BadRequest(
                    "`match:ignore` is not a link; remove it as a posting tag".to_string(),
                ));
            }
            write_one(
                &state,
                &halves,
                |journal, refs| removal(journal, refs, MATCH_TAG, &id, matching::unlink_ops),
                |journal, id, _| removed(journal, MATCH_TAG, id),
            )
        })
    })?;
    Ok(ok(body))
}

/// `POST /api/matches/forget` — `{value, halves}`: "Forget rejection" (D12
/// Undo), removing `nomatch:VALUE` from every posting carrying it. Errors as
/// [`unlink`].
pub(crate) async fn forget(
    State(state): State<AppState>,
    payload: Result<Json<ForgetIn>, JsonRejection>,
) -> Result<Response, AppError> {
    let request = json_body(payload)?;
    let value = request.value.clone();
    let body = redacted(&state, {
        tagged_halves(&request.halves, NOMATCH_TAG, &value).and_then(|halves| {
            write_one(
                &state,
                &halves,
                |journal, refs| removal(journal, refs, NOMATCH_TAG, &value, matching::forget_ops),
                |journal, value, _| removed(journal, NOMATCH_TAG, value),
            )
        })
    })?;
    Ok(ok(body))
}

/// `POST /api/matches/ignore` — `{half}`: "Ignore this side" (D1, D9),
/// writing `match:ignore` and cleaning up the posting's rejections as a link
/// does. Errors as [`link`].
pub(crate) async fn ignore(
    State(state): State<AppState>,
    payload: Result<Json<IgnoreIn>, JsonRejection>,
) -> Result<Response, AppError> {
    let request = json_body(payload)?;
    let body = redacted(
        &state,
        write_one(
            &state,
            &[&request.half],
            |journal, refs| {
                let ops = matching::ignore_ops(journal, refs[0]).map_err(match_error)?;
                Ok((IGNORE_VALUE.to_string(), ops))
            },
            |journal, id, refs| carries(journal, refs[0], MATCH_TAG, id),
        ),
    )?;
    Ok(ok(body))
}

/// `POST /api/matches/link-all` — `{pairs: [{a, b}]}`: "Link all clear
/// matches" (D9).
///
/// Every half's fingerprint is checked first and every pair is validated
/// against the journal as it stands, so a stale or illegal pair refuses the
/// whole request before anything is written. The links are then written in
/// batches of at most [`MAX_BATCH`] transactions, each one lock-held apply
/// and one save, the whole run under one lock. No posting may be in two
/// pairs.
///
/// `200` `{generation, linked, matchIds}` (ids in request order).
pub(crate) async fn link_all(
    State(state): State<AppState>,
    payload: Result<Json<LinkAllIn>, JsonRejection>,
) -> Result<Response, AppError> {
    let request = json_body(payload)?;
    let body = redacted(&state, link_all_locked(&state, &request.pairs))?;
    Ok(ok(body))
}

fn link_all_locked(state: &AppState, pairs: &[PairIn]) -> Result<Bytes, AppError> {
    if pairs.is_empty() || pairs.len() > MAX_BULK_PAIRS {
        return Err(AppError::BadRequest(format!(
            "link 1 to {MAX_BULK_PAIRS} pairs, got {}",
            pairs.len()
        )));
    }
    let halves: Vec<&HalfIn> = pairs.iter().flat_map(pair_halves).collect();
    let mut seen = HashSet::new();
    if let Some(twice) = halves
        .iter()
        .find(|half| !seen.insert((half.index, half.posting)))
    {
        return Err(AppError::BadRequest(format!(
            "posting #{} of transaction #{} is in more than one pair",
            twice.posting, twice.index
        )));
    }

    let mut guard = lock_editor(state)?;
    let editor = bound(&mut guard)?;
    let journal = Arc::clone(editor.journal());
    let refs = resolve(editor, &journal, &halves)?;
    let pairs: Vec<[(&HalfIn, PostingRef); 2]> = halves
        .as_chunks::<2>()
        .0
        .iter()
        .zip(refs.as_chunks::<2>().0)
        .map(|([half_a, half_b], [a, b])| [(*half_a, *a), (*half_b, *b)])
        .collect();
    // Every pair legal before anything is written, so a bad one cannot leave
    // a half-linked journal behind it.
    let mut ids = MatchIds::new(&journal);
    let match_ids: Vec<String> = pairs.iter().map(|_| ids.next_id()).collect();
    {
        let linker = Linker::new(&journal);
        for ([(_, a), (_, b)], id) in pairs.iter().zip(&match_ids) {
            linker.link_ops(*a, *b, id).map_err(match_error)?;
        }
    }

    let mut next = 0;
    while next < pairs.len() {
        let editor = bound(&mut guard)?;
        let journal = Arc::clone(editor.journal());
        let linker = Linker::new(&journal);
        let mut merged = Merged::default();
        let start = next;
        while next < pairs.len() {
            let [(half_a, a), (half_b, b)] = pairs[next];
            let ops = linker
                .link_ops(a, b, &match_ids[next])
                .map_err(match_error)?;
            // A pair always fits an empty batch; otherwise stop before this
            // one would take the batch past `MAX_BATCH` transactions.
            let mut candidate = Merged(merged.0.clone());
            candidate.add(ops);
            candidate.describe(&journal, &[(half_a, a), (half_b, b)]);
            if candidate.len() > MAX_BATCH && next > start {
                break;
            }
            merged = candidate;
            next += 1;
        }
        merged.apply(editor)?;
        let written = editor.journal();
        let index = matching::links(written);
        let all_linked =
            pairs[start..next]
                .iter()
                .zip(&match_ids[start..next])
                .all(|([(_, a), (_, b)], id)| {
                    index
                        .get(id)
                        .is_some_and(|pair| pair == (*a, *b) || pair == (*b, *a))
                });
        if !all_linked {
            resync_from_disk(state, &mut guard)?;
            return Err(AppError::Internal(format!(
                "linking did not read back as asked; {start} of {} pairs were saved",
                pairs.len()
            )));
        }
        save_and_publish(state, &mut guard)?;
    }
    let snapshot = state.snapshot();
    drop(guard);
    to_bytes(&LinkAllBody {
        generation: generation_of(&snapshot.etag),
        linked: pairs.len(),
        match_ids,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_default_to_d8_and_refuse_out_of_range() {
        let settings = match_settings(None, None).expect("defaults");
        assert_eq!((settings.window_days, settings.lookback_days), (10, 365));
        assert!(match_settings(Some(0), None).is_err());
        assert!(match_settings(Some(61), None).is_err());
        assert!(match_settings(Some(60), Some(0)).is_ok());
        assert!(match_settings(None, Some(36_501)).is_err());
    }

    #[test]
    fn merged_ops_are_queued_once_per_transaction() {
        let op = EditOp::RemovePostingTagValue {
            posting: 0,
            name: NOMATCH_TAG.to_string(),
            value: "abc123".to_string(),
        };
        let mut merged = Merged::default();
        merged.add(vec![(Tindex(3), vec![op.clone()])]);
        merged.add(vec![(Tindex(3), vec![op.clone()]), (Tindex(1), vec![op])]);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged.0.get(&3).map(Vec::len), Some(1));
        assert_eq!(merged.0.keys().copied().collect::<Vec<_>>(), vec![1, 3]);
    }
}
