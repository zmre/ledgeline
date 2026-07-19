//! Native (non-hledger) WRITE endpoints — the Phase 5.2 wiring of the journal
//! write path ([`ledgeline_core::edit::JournalEditor`]) into axum.
//!
//! Two routes mutate the on-disk journal file, each serializing on the shared
//! editor mutex ([`AppState::editor`]) and, on success, rebuilding + republishing
//! the read snapshot so `GET /transactions` and the `/api/*` reports reflect the
//! change immediately:
//! - `POST   /api/transactions`         — add a transaction from a native body.
//! - `DELETE /api/transactions/{index}` — delete the transaction with that
//!   `tindex`.
//!
//! # JSON contract (native, camelCase, mirroring the SPA)
//! An amount's exact quantity uses the same `Dec` shape as the report endpoints:
//! `{ "mantissa": "<base-10 string>", "places": <number> }` (string mantissa so a
//! large computed value never loses precision through a JS number).
//!
//! `POST /api/transactions` request:
//! ```json
//! {
//!   "date": "2026-07-20",
//!   "status": "cleared",                 // optional: cleared|pending|unmarked
//!   "code": "INV-9",                     // optional
//!   "description": "Safeway | groceries",// optional
//!   "position": "append",                // optional: append|dateOrdered (default append)
//!   "postings": [
//!     { "account": "expenses:food:groceries",
//!       "amount": { "commodity": "$", "quantity": { "mantissa": "5624", "places": 2 } } },
//!     { "account": "liabilities:cc:visa" } // no amount ⇒ the elided/inferred leg
//!   ]
//! }
//! ```
//! A posting `amount` may also carry a `cost`:
//! `{ "kind": "unit"|"total", "amount": { "commodity": "$", "quantity": <Dec> } }`.
//!
//! # Amount style inference (correctness-critical)
//! The editor renders each amount through its [`AmountStyle`] and then re-parses
//! to validate the round-trip, so a wrong decimal mark (e.g. rendering a EUR
//! amount with `.` when the journal uses `,`) is a silent value corruption the
//! guard would reject. We therefore INFER each amount's style from the journal:
//! the commodity's declared canonical style if present, else the style of the
//! first existing amount in that commodity, else a sensible default (a symbol-only
//! commodity like `$` on the left/unspaced, an alphabetic code like `EUR` on the
//! right/spaced, `.` decimal mark). This makes the formatted amount match the
//! journal's conventions AND pass the editor's round-trip / decimal-mark guard.
//!
//! # `EditError` → HTTP
//! `ExternalChange` → `409`, `Unbalanced`/`Unsupported`/`ParseInvalidAfterEdit`/
//! `RoundTripMismatch` → `400`, `TransactionNotFound` → `404`, `Io`/`Parse`/
//! `Decimal`/`Internal` → `500`. A `409` means the file changed under us; the
//! client should re-fetch and retry.

use std::sync::{MutexGuard, PoisonError};

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use ledgeline_core::edit::InsertPosition;
use ledgeline_core::model::{
    AccountName, Amount, AmountStyle, Commodity, CommoditySide, Cost, CostKind, Journal, Posting,
    PostingType, SourcePos, Status, Tindex, Transaction,
};
use ledgeline_core::{Dec, EditError, JournalEditor};
use serde::{Deserialize, Serialize};

use crate::AppState;

/// An HTTP error: a status plus a human-readable message (mirrors `reports_api`).
type ApiError = (StatusCode, String);

// ===========================================================================
// Request body
// ===========================================================================

/// An exact decimal on the wire: `mantissa / 10^places`, mantissa STRING-encoded.
#[derive(Deserialize)]
struct WireDecIn {
    mantissa: String,
    places: u32,
}

/// The priced side of a cost annotation: a bare commodity + quantity.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PricedAmountIn {
    commodity: String,
    quantity: WireDecIn,
}

/// A `@`/`@@` cost annotation on a posting amount.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CostIn {
    kind: CostKindIn,
    amount: PricedAmountIn,
}

/// A single-commodity posting amount, optionally with a cost.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AmountIn {
    commodity: String,
    quantity: WireDecIn,
    #[serde(default)]
    cost: Option<CostIn>,
}

/// One posting: an account and an optional amount. No `amount` marks the elided
/// leg whose value the parser infers from the balance.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PostingIn {
    account: String,
    #[serde(default)]
    amount: Option<AmountIn>,
}

/// The `POST /api/transactions` request body.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AddRequest {
    date: String,
    #[serde(default)]
    status: Option<StatusIn>,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    description: Option<String>,
    postings: Vec<PostingIn>,
    #[serde(default)]
    position: Option<PositionIn>,
}

impl AddRequest {
    /// The insert position, defaulting to `Append` (predictable end-of-file
    /// placement that leaves every existing transaction byte-identical).
    fn insert_position(&self) -> InsertPosition {
        match self.position {
            Some(PositionIn::DateOrdered) => InsertPosition::DateOrdered,
            Some(PositionIn::Append) | None => InsertPosition::Append,
        }
    }
}

/// Transaction clearing status on the wire.
#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum StatusIn {
    Cleared,
    Pending,
    Unmarked,
}

impl From<StatusIn> for Status {
    fn from(status: StatusIn) -> Self {
        match status {
            StatusIn::Cleared => Status::Cleared,
            StatusIn::Pending => Status::Pending,
            StatusIn::Unmarked => Status::Unmarked,
        }
    }
}

/// Cost kind on the wire (`@` unit vs `@@` total).
#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum CostKindIn {
    Unit,
    Total,
}

impl From<CostKindIn> for CostKind {
    fn from(kind: CostKindIn) -> Self {
        match kind {
            CostKindIn::Unit => CostKind::Unit,
            CostKindIn::Total => CostKind::Total,
        }
    }
}

/// Where to place the new transaction.
#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
enum PositionIn {
    Append,
    DateOrdered,
}

// ===========================================================================
// Response body
// ===========================================================================

/// An exact decimal on the wire (string mantissa; see [`WireDecIn`]).
#[derive(Serialize)]
struct WireDecOut {
    mantissa: String,
    places: u32,
}

fn wire_dec_out(dec: Dec) -> WireDecOut {
    WireDecOut {
        mantissa: dec.mantissa.to_string(),
        places: dec.places,
    }
}

/// A serialized cost annotation.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeCost {
    kind: &'static str,
    amount: NativeAmount,
}

/// A serialized single-commodity amount (with any cost).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeAmount {
    commodity: String,
    quantity: WireDecOut,
    #[serde(skip_serializing_if = "Option::is_none")]
    cost: Option<Box<NativeCost>>,
}

/// A serialized posting: account plus its (possibly inferred) amounts.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NativePosting {
    account: String,
    amounts: Vec<NativeAmount>,
    status: &'static str,
    #[serde(rename = "type")]
    ptype: &'static str,
}

/// A serialized transaction as it landed in the journal after the reparse (its
/// `index` is the reassigned file-order `tindex`, its elided leg now filled in).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeTransaction {
    index: u32,
    date: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    date2: Option<String>,
    status: &'static str,
    code: String,
    description: String,
    postings: Vec<NativePosting>,
}

/// The `POST /api/transactions` 201 response: the added transaction + its index.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AddResponse {
    index: u32,
    transaction: NativeTransaction,
}

/// The `DELETE /api/transactions/{index}` 200 response.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeleteResponse {
    deleted_index: u32,
    remaining: usize,
}

// ===========================================================================
// Handlers
// ===========================================================================

/// `POST /api/transactions` — add a transaction from a native JSON body.
///
/// Builds a [`Transaction`] (inferring each amount's style from the journal),
/// adds it through the editor, saves atomically, republishes the snapshot, and
/// returns `201` with the added transaction.
pub(crate) async fn add_transaction(
    State(state): State<AppState>,
    payload: Result<Json<AddRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<AddResponse>), ApiError> {
    let Json(request) = payload.map_err(|rejection| {
        (
            StatusCode::BAD_REQUEST,
            format!("invalid request body: {rejection}"),
        )
    })?;
    // All editing work is synchronous and holds the std mutex only inside this
    // call — never across an `.await` — so the guard never crosses a yield point.
    let response = add_transaction_locked(&state, &request)?;
    Ok((StatusCode::CREATED, Json(response)))
}

/// `DELETE /api/transactions/{index}` — delete the transaction with that
/// `tindex`, save, republish, and return `200`.
pub(crate) async fn delete_transaction(
    State(state): State<AppState>,
    Path(index): Path<u32>,
) -> Result<Json<DeleteResponse>, ApiError> {
    let response = delete_transaction_locked(&state, index)?;
    Ok(Json(response))
}

// ===========================================================================
// Locked, synchronous edit logic (no `.await` while the mutex is held)
// ===========================================================================

fn add_transaction_locked(state: &AppState, request: &AddRequest) -> Result<AddResponse, ApiError> {
    let mut guard = lock_editor(state);
    let editor = guard.as_mut().ok_or_else(editing_disabled)?;

    let transaction = build_transaction(editor.journal(), request)?;
    let position = request.insert_position();
    // Compute where the row will land BEFORE mutating (the reparse reassigns every
    // later tindex); this mirrors the editor's own `insertion_point` so we can
    // return the added transaction afterwards.
    let insert_pos = insertion_index(editor.journal(), &transaction, position);

    editor
        .add_transaction(&transaction, position)
        .map_err(edit_error)?;
    save_and_publish(state, editor).map_err(edit_error)?;

    let added = editor
        .journal()
        .transactions
        .get(insert_pos)
        .ok_or_else(|| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "could not locate the added transaction after saving".to_string(),
            )
        })?;
    Ok(AddResponse {
        index: added.index.0,
        transaction: native_transaction(added),
    })
}

fn delete_transaction_locked(state: &AppState, index: u32) -> Result<DeleteResponse, ApiError> {
    let mut guard = lock_editor(state);
    let editor = guard.as_mut().ok_or_else(editing_disabled)?;

    editor
        .delete_transaction(Tindex(index))
        .map_err(edit_error)?;
    save_and_publish(state, editor).map_err(edit_error)?;

    Ok(DeleteResponse {
        deleted_index: index,
        remaining: editor.transaction_count(),
    })
}

/// Save the editor's pending edit and republish the read snapshot.
///
/// On success the snapshot is rebuilt from the edited journal. On ANY save
/// failure (notably [`EditError::ExternalChange`]) the in-memory edit is
/// unpersisted, so we re-open the editor from disk — discarding that edit and
/// re-syncing the rope/fingerprint — and publish the on-disk state, so the editor
/// and the served snapshot stay consistent with the file. The original save error
/// is returned for the caller to map (a `409` tells the client to re-fetch/retry).
fn save_and_publish(state: &AppState, editor: &mut JournalEditor) -> Result<(), EditError> {
    let result = editor.save();
    if result.is_err()
        && let Ok(reopened) = JournalEditor::open(editor.path().to_path_buf())
    {
        *editor = reopened;
    }
    state.replace_journal(editor.journal());
    result
}

/// Lock the editor mutex, recovering from poisoning (a prior panic mid-edit) by
/// taking the inner value rather than propagating the panic across every future
/// request.
fn lock_editor(state: &AppState) -> MutexGuard<'_, Option<JournalEditor>> {
    state
        .editor()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// The `501` returned when this state has no editor (built from a parsed journal
/// with no backing file).
fn editing_disabled() -> ApiError {
    (
        StatusCode::NOT_IMPLEMENTED,
        "editing is not enabled: this server was started without a journal file bound to an editor"
            .to_string(),
    )
}

/// Map an [`EditError`] onto its HTTP status + message.
fn edit_error(error: EditError) -> ApiError {
    let status = match error {
        EditError::ExternalChange => StatusCode::CONFLICT,
        EditError::Unbalanced
        | EditError::Unsupported(_)
        | EditError::ParseInvalidAfterEdit(_)
        | EditError::RoundTripMismatch => StatusCode::BAD_REQUEST,
        EditError::TransactionNotFound(_) => StatusCode::NOT_FOUND,
        EditError::Io(_) | EditError::Parse(_) | EditError::Decimal(_) | EditError::Internal(_) => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
    };
    (status, error.to_string())
}

/// The 0-based file-order position the new transaction will occupy — mirrors
/// [`JournalEditor`]'s `insertion_point`: append at end, or (date-ordered) before
/// the first existing transaction whose date is strictly later.
fn insertion_index(journal: &Journal, txn: &Transaction, position: InsertPosition) -> usize {
    let len = journal.transactions.len();
    match position {
        InsertPosition::Append => len,
        InsertPosition::DateOrdered => journal
            .transactions
            .iter()
            .position(|existing| existing.date.as_str() > txn.date.as_str())
            .unwrap_or(len),
    }
}

// ===========================================================================
// Build a `core::Transaction` from the request (with inferred styles)
// ===========================================================================

fn build_transaction(journal: &Journal, request: &AddRequest) -> Result<Transaction, ApiError> {
    if request.date.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "a transaction needs a date".to_string(),
        ));
    }
    if request.postings.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "a transaction needs at least one posting".to_string(),
        ));
    }
    let postings = request
        .postings
        .iter()
        .map(|posting| build_posting(journal, posting))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Transaction {
        // Placeholder; the editor reassigns file-order indices on reparse.
        index: Tindex(0),
        date: request.date.clone(),
        date2: None,
        status: request.status.map_or(Status::Unmarked, Status::from),
        code: request.code.clone().unwrap_or_default(),
        description: request.description.clone().unwrap_or_default(),
        comment: String::new(),
        preceding_comment: String::new(),
        tags: Vec::new(),
        postings,
        // Placeholder; recomputed on reparse.
        source_span: (
            SourcePos { line: 1, column: 1 },
            SourcePos { line: 1, column: 1 },
        ),
    })
}

fn build_posting(journal: &Journal, input: &PostingIn) -> Result<Posting, ApiError> {
    if input.account.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "a posting needs an account".to_string(),
        ));
    }
    let amounts = match &input.amount {
        Some(amount) => vec![build_amount(journal, amount)?],
        None => Vec::new(),
    };
    Ok(Posting {
        status: Status::Unmarked,
        ptype: PostingType::Regular,
        account: AccountName(input.account.clone()),
        amounts,
        balance_assertion: None,
        date: None,
        date2: None,
        comment: String::new(),
        tags: Vec::new(),
    })
}

fn build_amount(journal: &Journal, input: &AmountIn) -> Result<Amount, ApiError> {
    let commodity = Commodity(input.commodity.clone());
    let quantity = dec_from_wire(&input.quantity)?;
    let style = infer_style(journal, &commodity, quantity.places);
    let cost = match &input.cost {
        Some(cost) => Some(Box::new(build_cost(journal, cost)?)),
        None => None,
    };
    Ok(Amount {
        commodity,
        quantity,
        style,
        cost,
    })
}

fn build_cost(journal: &Journal, input: &CostIn) -> Result<Cost, ApiError> {
    let commodity = Commodity(input.amount.commodity.clone());
    let quantity = dec_from_wire(&input.amount.quantity)?;
    let style = infer_style(journal, &commodity, quantity.places);
    Ok(Cost {
        kind: input.kind.into(),
        amount: Amount {
            commodity,
            quantity,
            style,
            cost: None,
        },
    })
}

fn dec_from_wire(dec: &WireDecIn) -> Result<Dec, ApiError> {
    let mantissa = dec.mantissa.trim().parse::<i128>().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            format!(
                "invalid amount mantissa '{}': expected a base-10 integer string",
                dec.mantissa
            ),
        )
    })?;
    Ok(Dec::new(mantissa, dec.places))
}

// ===========================================================================
// Amount-style inference
// ===========================================================================

/// Infer the display style for `commodity`: its declared canonical style, else
/// the style of the first existing amount in that commodity anywhere in the
/// journal, else a sensible default. The side/spacing/decimal-mark this yields is
/// what makes the formatted amount re-parse to the same value (and pass the
/// editor's round-trip guard).
fn infer_style(journal: &Journal, commodity: &Commodity, places: u32) -> AmountStyle {
    find_style_for(journal, commodity).unwrap_or_else(|| default_style(commodity, places))
}

fn find_style_for(journal: &Journal, commodity: &Commodity) -> Option<AmountStyle> {
    // 1. The declared canonical style (a `commodity`/`D` directive).
    if let Some((_, style)) = journal
        .commodity_styles
        .iter()
        .find(|(declared, _)| declared == commodity)
    {
        return Some(style.clone());
    }
    // 2. The first amount in this commodity anywhere: posting amounts (and any
    //    nested cost amounts), balance assertions, and price directives.
    let mut amounts: Vec<&Amount> = Vec::new();
    for txn in &journal.transactions {
        for posting in &txn.postings {
            for amount in &posting.amounts {
                collect_amounts(amount, &mut amounts);
            }
            if let Some(assertion) = &posting.balance_assertion {
                collect_amounts(&assertion.amount, &mut amounts);
            }
        }
    }
    for price in &journal.prices {
        collect_amounts(&price.price, &mut amounts);
    }
    amounts
        .into_iter()
        .find(|amount| &amount.commodity == commodity)
        .map(|amount| amount.style.clone())
}

/// Push `amount` and every amount nested in its cost chain onto `out`.
fn collect_amounts<'a>(amount: &'a Amount, out: &mut Vec<&'a Amount>) {
    out.push(amount);
    if let Some(cost) = &amount.cost {
        collect_amounts(&cost.amount, out);
    }
}

/// A default style for a commodity the journal has never seen: a symbol-only
/// commodity (e.g. `$`) renders on the left with no space; an alphabetic code
/// (e.g. `EUR`, `AAPL`) on the right, spaced. `.` decimal mark round-trips.
fn default_style(commodity: &Commodity, places: u32) -> AmountStyle {
    let symbol_only =
        !commodity.0.is_empty() && commodity.0.chars().all(|c| !c.is_ascii_alphanumeric());
    let (side, spaced) = if symbol_only {
        (CommoditySide::Left, false)
    } else {
        (CommoditySide::Right, true)
    };
    AmountStyle {
        side,
        spaced,
        decimal_mark: Some('.'),
        digit_groups: None,
        precision: places,
    }
}

// ===========================================================================
// core -> native response mapping
// ===========================================================================

fn native_transaction(txn: &Transaction) -> NativeTransaction {
    NativeTransaction {
        index: txn.index.0,
        date: txn.date.clone(),
        date2: txn.date2.clone(),
        status: status_str(txn.status),
        code: txn.code.clone(),
        description: txn.description.clone(),
        postings: txn.postings.iter().map(native_posting).collect(),
    }
}

fn native_posting(posting: &Posting) -> NativePosting {
    NativePosting {
        account: posting.account.0.clone(),
        amounts: posting.amounts.iter().map(native_amount).collect(),
        status: status_str(posting.status),
        ptype: ptype_str(posting.ptype),
    }
}

fn native_amount(amount: &Amount) -> NativeAmount {
    NativeAmount {
        commodity: amount.commodity.0.clone(),
        quantity: wire_dec_out(amount.quantity),
        cost: amount.cost.as_ref().map(|cost| {
            Box::new(NativeCost {
                kind: costkind_str(cost.kind),
                amount: native_amount(&cost.amount),
            })
        }),
    }
}

fn status_str(status: Status) -> &'static str {
    match status {
        Status::Unmarked => "unmarked",
        Status::Pending => "pending",
        Status::Cleared => "cleared",
    }
}

fn ptype_str(ptype: PostingType) -> &'static str {
    match ptype {
        PostingType::Regular => "regular",
        PostingType::Virtual => "virtual",
        PostingType::BalancedVirtual => "balancedVirtual",
    }
}

fn costkind_str(kind: CostKind) -> &'static str {
    match kind {
        CostKind::Unit => "unit",
        CostKind::Total => "total",
    }
}
