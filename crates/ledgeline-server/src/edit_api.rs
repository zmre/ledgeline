//! Native (non-hledger) WRITE endpoints — the Phase 5.2 wiring of the journal
//! write path ([`ledgeline_core::edit::JournalEditor`]) into axum.
//!
//! These routes mutate the on-disk journal file, each serializing on the shared
//! editor mutex ([`AppState::editor`]) and, on success, rebuilding + republishing
//! the read snapshot so `GET /transactions` and the `/api/*` reports reflect the
//! change immediately:
//! - `POST   /api/transactions`         — add a transaction from a native body.
//! - `DELETE /api/transactions/{index}` — delete the transaction with that
//!   `tindex`.
//! - `PUT    /api/transactions/{index}` — full, in-place replace of that
//!   transaction (the edit popup); body is the [`AddRequest`] shape (with
//!   optional transaction/posting `comment`s so the replace round-trips them).
//! - `PATCH  /api/transactions/{index}` — surgical partial edit (inline edits):
//!   `{ "description"?, "status"?, "postings"?: [{ "index", "account" }] }`, each
//!   touching only its own field on disk.
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
//!   "date2": "2026-07-22",               // optional: secondary date (DATE=DATE2)
//!   "status": "cleared",                 // optional: cleared|pending|unmarked
//!   "code": "INV-9",                     // optional
//!   "description": "Safeway | groceries",// optional
//!   "comment": "category:food",          // optional: rides the header (carries tags)
//!   "position": "append",                // optional: append|dateOrdered (default dateOrdered)
//!   "postings": [
//!     { "account": "expenses:food:groceries",
//!       "status": "pending",             // optional per-posting: cleared|pending|unmarked
//!       "amount": { "commodity": "$", "quantity": { "mantissa": "5624", "places": 2 } } },
//!     { "account": "liabilities:cc:visa" } // no amount ⇒ the elided/inferred leg
//!   ]
//! }
//! ```
//! A posting `amount` may also carry a `cost`:
//! `{ "kind": "unit"|"total", "amount": { "commodity": "$", "quantity": <Dec> } }`.
//!
//! A posting may also carry `type` and `balanceAssertion`:
//! ```json
//! { "account": "assets:cash",
//!   "type": "regular",                   // regular|virtual|balancedVirtual (default regular)
//!   "amount": { "commodity": "$", "quantity": { "mantissa": "-100", "places": 2 } },
//!   "balanceAssertion": {                // the `= AMOUNT` reconciliation anchor
//!     "amount": { "commodity": "$", "quantity": { "mantissa": "9900", "places": 2 } },
//!     "total": false,                    // true ⇒ `==` (this commodity ONLY)
//!     "inclusive": false                 // true ⇒ `=*` (include subaccounts)
//!   } }
//! ```
//! Both are OPTIONAL but NOT defaulted-away on a replace: a `PUT` that omits
//! them writes a posting that genuinely has neither. That is DL-2 — these
//! fields did not exist, so every `PUT` erased balance assertions and rewrote
//! `[balanced-virtual]` and `(virtual)` postings as real ones, with a `200`.
//! Any client doing a read-modify-write must echo them back, as the SPA does.
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
//!
//! Every one of those bodies goes out through [`redacted`] first, which rewrites
//! the journal's own absolute paths down to bare file names (SEC-15). A parse
//! failure renders as `{source_name}:{line}: …`, and `source_name` is the
//! absolute path the editor was opened with — see [`PathRedaction`] for what is
//! redacted, what deliberately is not, and why.

use std::sync::MutexGuard;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use ledgeline_core::decimal::MAX_PARSE_PLACES;
use ledgeline_core::edit::InsertPosition;
use ledgeline_core::model::{
    AccountName, Amount, AmountStyle, BalanceAssertion, Commodity, CommoditySide, Cost, CostKind,
    Journal, Posting, PostingType, SourcePos, Status, Tindex, Transaction,
};
use ledgeline_core::{Dec, EditError, JournalEditor};
use serde::{Deserialize, Serialize};

use crate::AppState;
// `editing_disabled` lives in `error` rather than here because the rules-file
// `PUT` answers with the identical sentence, and the SPA matches on the words.
use crate::error::{AppError, editing_disabled};
use crate::reports_api::WireDec;

// ===========================================================================
// Request body
// ===========================================================================

/// An exact decimal on the wire: `mantissa / 10^places`, mantissa STRING-encoded.
///
/// `pub(crate)` for [`crate::budget_api`], which takes the same shape for a
/// budget goal and must reject the same absurd values through the same
/// [`dec_from_wire`].
#[derive(Deserialize)]
pub(crate) struct WireDecIn {
    pub(crate) mantissa: String,
    pub(crate) places: u32,
}

/// A bare commodity + quantity, with no cost of its own: the priced side of a
/// cost annotation, and the asserted side of a balance assertion.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PricedAmountIn {
    commodity: String,
    quantity: WireDecIn,
}

/// A `=` / `==` / `=*` / `==*` balance assertion on a posting — the
/// reconciliation anchor that pins an account's running balance at that point.
///
/// The two flags pick the operator, exactly as the model stores them: `total`
/// asserts the account holds ONLY this commodity (`==`), `inclusive` includes
/// subaccounts (`=*`). Both default to `false`, i.e. a plain `=`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BalanceAssertionIn {
    amount: PricedAmountIn,
    #[serde(default)]
    inclusive: bool,
    #[serde(default)]
    total: bool,
}

/// Real / unbalanced-virtual / balanced-virtual on the wire (hledger's `ptype`).
///
/// The spellings are exactly the ones this module SERIALIZES (see [`ptype_str`]),
/// so a posting the API handed out round-trips straight back through it. An
/// unrecognized value is rejected by serde as an unknown variant, which the
/// handlers surface as a `400` — never a silent fallback to `Regular`, which is
/// what DL-2 was: a `[budget:env]` envelope posting quietly rewritten as a real
/// one, moving money onto the balance sheet that was never there.
#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
enum PostingTypeIn {
    Regular,
    Virtual,
    BalancedVirtual,
}

impl From<PostingTypeIn> for PostingType {
    fn from(ptype: PostingTypeIn) -> Self {
        match ptype {
            PostingTypeIn::Regular => PostingType::Regular,
            PostingTypeIn::Virtual => PostingType::Virtual,
            PostingTypeIn::BalancedVirtual => PostingType::BalancedVirtual,
        }
    }
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
/// leg whose value the parser infers from the balance. An optional `comment`
/// carries a same-line posting comment (so a full replace round-trips it).
///
/// `type` and `balanceAssertion` exist so a REPLACE cannot silently destroy
/// either (DL-2). Both are optional for backwards compatibility, but "absent"
/// means "this posting has none" — so a client that reads a transaction and
/// writes it back MUST echo them, exactly as it already echoes `comment`. The
/// SPA does this in `editMapping.ts`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PostingIn {
    account: String,
    #[serde(default)]
    status: Option<StatusIn>,
    #[serde(default)]
    amount: Option<AmountIn>,
    #[serde(default)]
    comment: Option<String>,
    /// Real / virtual `(a)` / balanced-virtual `[a]`; absent = `regular`.
    #[serde(default, rename = "type")]
    ptype: Option<PostingTypeIn>,
    /// The `= AMOUNT` reconciliation anchor; absent = no assertion.
    #[serde(default)]
    balance_assertion: Option<BalanceAssertionIn>,
}

/// The `POST /api/transactions` (add) and `PUT /api/transactions/{index}`
/// (full replace) request body. The optional `comment` (transaction-level) and
/// per-posting `comment` let a replace round-trip comments; `POST` clients that
/// omit them add a comment-free transaction (unchanged behavior).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AddRequest {
    date: String,
    #[serde(default)]
    date2: Option<String>,
    #[serde(default)]
    status: Option<StatusIn>,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    comment: Option<String>,
    postings: Vec<PostingIn>,
    #[serde(default)]
    position: Option<PositionIn>,
}

/// The `PATCH /api/transactions/{index}` request body: a surgical partial edit
/// applying an optional new `description` and/or any number of per-posting
/// account changes. Fields left out are unchanged.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PatchRequest {
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    status: Option<StatusIn>,
    #[serde(default)]
    postings: Vec<PostingPatch>,
}

/// One entry of a [`PatchRequest`]: set posting `index`'s account to `account`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PostingPatch {
    index: usize,
    account: String,
}

impl AddRequest {
    /// The insert position, defaulting to `DateOrdered` (place the row next to its
    /// chronological neighbors, across `include`d files) when the request omits a
    /// position. `Append` remains available for callers that ask for it explicitly.
    fn insert_position(&self) -> InsertPosition {
        match self.position {
            Some(PositionIn::Append) => InsertPosition::Append,
            Some(PositionIn::DateOrdered) | None => InsertPosition::DateOrdered,
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

// The RESPONSE decimal is `reports_api::WireDec` (imported above), not a local
// copy. This module used to carry a byte-identical `WireDecOut` + `wire_dec_out`
// pair (DRY-4); sharing the one type is what keeps the read and write wires from
// ever describing a decimal differently.

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
    quantity: WireDec,
    #[serde(skip_serializing_if = "Option::is_none")]
    cost: Option<Box<NativeCost>>,
}

/// A serialized balance assertion (the `= AMOUNT` anchor), in the same shape
/// [`BalanceAssertionIn`] accepts so the response round-trips back as a request.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeBalanceAssertion {
    amount: NativeAmount,
    inclusive: bool,
    total: bool,
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
    /// Absent when the posting has no assertion — so the response says exactly
    /// what landed in the file, and a client can echo it straight back.
    #[serde(skip_serializing_if = "Option::is_none")]
    balance_assertion: Option<NativeBalanceAssertion>,
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

/// Unwrap a JSON request body, turning a malformed/absent one into the `400`
/// all three body-taking handlers used to spell out identically.
///
/// The message text is part of the contract: `native.ts` surfaces it verbatim
/// as a `ValidationError`, and `tests/error_surface.rs` pins it.
pub(crate) fn json_body<T>(payload: Result<Json<T>, JsonRejection>) -> Result<T, AppError> {
    payload
        .map(|Json(body)| body)
        .map_err(|rejection| AppError::BadRequest(format!("invalid request body: {rejection}")))
}

/// `POST /api/transactions` — add a transaction from a native JSON body.
///
/// Builds a [`Transaction`] (inferring each amount's style from the journal),
/// adds it through the editor, saves atomically, republishes the snapshot, and
/// returns `201` with the added transaction.
pub(crate) async fn add_transaction(
    State(state): State<AppState>,
    payload: Result<Json<AddRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<AddResponse>), AppError> {
    let request = json_body(payload)?;
    // All editing work is synchronous and holds the std mutex only inside this
    // call — never across an `.await` — so the guard never crosses a yield point.
    let response = redacted(&state, add_transaction_locked(&state, &request))?;
    Ok((StatusCode::CREATED, Json(response)))
}

/// `DELETE /api/transactions/{index}` — delete the transaction with that
/// `tindex`, save, republish, and return `200`.
pub(crate) async fn delete_transaction(
    State(state): State<AppState>,
    Path(index): Path<u32>,
) -> Result<Json<DeleteResponse>, AppError> {
    let response = redacted(&state, delete_transaction_locked(&state, index))?;
    Ok(Json(response))
}

/// `PUT /api/transactions/{index}` — full, in-place replace of the transaction
/// with that `tindex` (the edit popup's save).
///
/// Builds a [`Transaction`] from the (comment-carrying) [`AddRequest`] body,
/// replaces the addressed transaction in place through the editor, saves, and
/// republishes; returns `200` with the updated transaction.
pub(crate) async fn replace_transaction(
    State(state): State<AppState>,
    Path(index): Path<u32>,
    payload: Result<Json<AddRequest>, JsonRejection>,
) -> Result<Json<AddResponse>, AppError> {
    let request = json_body(payload)?;
    let response = redacted(&state, replace_transaction_locked(&state, index, &request))?;
    Ok(Json(response))
}

/// `PATCH /api/transactions/{index}` — surgical partial edit (inline edits):
/// apply an optional new description and/or per-posting account changes, each
/// touching only its own field on disk.
///
/// Saves + republishes only when at least one change was requested, then returns
/// `200` with the (possibly unchanged) transaction.
pub(crate) async fn patch_transaction(
    State(state): State<AppState>,
    Path(index): Path<u32>,
    payload: Result<Json<PatchRequest>, JsonRejection>,
) -> Result<Json<AddResponse>, AppError> {
    let request = json_body(payload)?;
    let response = redacted(&state, patch_transaction_locked(&state, index, &request))?;
    Ok(Json(response))
}

// ===========================================================================
// Locked, synchronous edit logic (no `.await` while the mutex is held)
// ===========================================================================

/// The bound editor inside a held guard, or the `501` that says this server has
/// none. Every locked operation below starts (and, after `save_and_publish`,
/// resumes) with this, so the "editing disabled" answer is written once.
fn bound(slot: &mut Option<JournalEditor>) -> Result<&mut JournalEditor, AppError> {
    slot.as_mut().ok_or_else(editing_disabled)
}

// ===========================================================================
// Path redaction (SEC-15)
// ===========================================================================

/// Rewrite the journal's own absolute paths out of an error body.
///
/// SEC-15. `EditError` renders through [`ParseError`]'s `Located` variant as
/// `{source_name}:{line}: {message}` — and `source_name` is the ABSOLUTE path
/// the editor was opened with, because that is what `JournalEditor::open` was
/// handed. So an edit that failed and could not be re-synced answered a `500`
/// naming exactly where the user keeps their money, and
/// [`EditError::ParseInvalidAfterEdit`] did the same in a `400`.
///
/// The rule the rest of the `/api` surface already holds to is that no response
/// body discloses an absolute path — `import_endpoints.rs` pins it for
/// `/api/import/*` and `tests/prefs.rs` for `/api/prefs`. This is the write
/// path holding to the same rule.
///
/// # What is NOT redacted, and why
///
/// The offending LINE of journal text. It is the single most useful thing in a
/// "your journal will not parse" message, and it is not a disclosure worth the
/// loss: anyone who can reach this endpoint holds the access token, and the
/// token already buys them `GET /transactions` — the whole journal, every line
/// of it. The absolute path is different in kind. It is not journal data at all;
/// it is a fact about the host that this API otherwise never states.
///
/// # Why this is not `import_api`'s `Redactor`
///
/// It should be. That type does the same job, with the same longest-first and
/// two-spellings rules, and having two is a duplication worth removing — but it
/// is private to `import_api`, and that file belongs to another lane right now.
/// Promoting it to a shared module is the follow-up.
struct PathRedaction {
    /// `(absolute spelling, replacement)`, applied longest-first.
    swaps: Vec<(String, String)>,
}

impl PathRedaction {
    /// The redaction for the journal `state` currently serves: every source file
    /// (main plus every `include`) mapped to its bare file name, and the main
    /// file's directory mapped away entirely.
    ///
    /// Both the path as recorded and its canonical spelling are registered,
    /// because they differ routinely — on macOS `/tmp` is a symlink to
    /// `/private/tmp`, and the editor and the parser can report either.
    fn for_journal(state: &AppState) -> Self {
        let sources = state.source_files();
        let files = sources.iter().flat_map(|path| {
            let handle = path.file_name().map_or_else(
                || "the journal".to_string(),
                |name| name.to_string_lossy().into_owned(),
            );
            spellings(path).map(move |spelling| (spelling, handle.clone()))
        });
        // The directory too: a *newly* added `include` naming a file that does
        // not exist is not in `source_files` (that parse never succeeded), and
        // `ParseError::Include` renders its absolute path.
        let dirs = sources
            .iter()
            .take(1)
            .filter_map(|path| path.parent())
            .flat_map(|dir| {
                spellings(dir).flat_map(|spelling| {
                    [
                        (format!("{spelling}/"), String::new()),
                        (spelling, ".".to_string()),
                    ]
                })
            });
        Self {
            swaps: files
                .chain(dirs)
                .filter(|(from, _)| !from.is_empty())
                .collect(),
        }
    }

    /// `text` with every known path rewritten. Longest-first, so a file's own
    /// entry wins over the directory prefix that also matches it.
    fn apply(&self, text: &str) -> String {
        let mut swaps: Vec<&(String, String)> = self.swaps.iter().collect();
        swaps.sort_by_key(|(from, _)| std::cmp::Reverse(from.len()));
        swaps.iter().fold(text.to_string(), |redacted, (from, to)| {
            redacted.replace(from.as_str(), to.as_str())
        })
    }

    /// `error` with its message redacted, keeping the variant — and therefore
    /// the status the SPA switches on — exactly as it was.
    ///
    /// The match is deliberately exhaustive rather than a catch-all: a new
    /// [`AppError`] variant must fail to compile here so somebody decides
    /// whether it can carry a path, instead of silently leaking one.
    fn redact(&self, error: AppError) -> AppError {
        match error {
            AppError::BadRequest(message) => AppError::BadRequest(self.apply(&message)),
            AppError::NotFound(message) => AppError::NotFound(self.apply(&message)),
            AppError::Conflict(message) => AppError::Conflict(self.apply(&message)),
            AppError::EditingDisabled(message) => AppError::EditingDisabled(self.apply(&message)),
            AppError::Unavailable(message) => AppError::Unavailable(self.apply(&message)),
            AppError::Internal(message) => AppError::Internal(self.apply(&message)),
        }
    }
}

/// A path as written and, when it differs, as canonicalized.
fn spellings(path: &std::path::Path) -> impl Iterator<Item = String> {
    let written = path.to_string_lossy().into_owned();
    let canonical = std::fs::canonicalize(path)
        .map(|resolved| resolved.to_string_lossy().into_owned())
        .ok()
        .filter(|resolved| *resolved != written);
    std::iter::once(written).chain(canonical)
}

/// Run one edit and redact the journal's paths out of whatever it failed with.
///
/// Applied at the four handler boundaries rather than at each `?`, so no future
/// failure inside the locked operations can bypass it (SEC-15). The redaction is
/// built only on the error path, so a successful edit pays nothing for it.
fn redacted<T>(state: &AppState, outcome: Result<T, AppError>) -> Result<T, AppError> {
    outcome.map_err(|error| PathRedaction::for_journal(state).redact(error))
}

fn add_transaction_locked(state: &AppState, request: &AddRequest) -> Result<AddResponse, AppError> {
    let mut guard = lock_editor(state)?;
    let editor = bound(&mut guard)?;

    let transaction = build_transaction(editor.journal(), request)?;
    let position = request.insert_position();
    // Compute where the row will land BEFORE mutating (the reparse reassigns every
    // later tindex); this mirrors the editor's own `insertion_point` so we can
    // return the added transaction afterwards.
    let insert_pos = insertion_index(editor.journal(), &transaction, position);

    editor.add_transaction(&transaction, position)?;
    save_and_publish(state, &mut guard)?;

    let editor = bound(&mut guard)?;
    let added = editor
        .journal()
        .transactions
        .get(insert_pos)
        .ok_or_else(|| {
            AppError::Internal("could not locate the added transaction after saving".to_string())
        })?;
    Ok(AddResponse {
        index: added.index.0,
        transaction: native_transaction(added),
    })
}

fn delete_transaction_locked(state: &AppState, index: u32) -> Result<DeleteResponse, AppError> {
    let mut guard = lock_editor(state)?;
    bound(&mut guard)?.delete_transaction(Tindex(index))?;
    save_and_publish(state, &mut guard)?;

    Ok(DeleteResponse {
        deleted_index: index,
        remaining: bound(&mut guard)?.transaction_count(),
    })
}

fn replace_transaction_locked(
    state: &AppState,
    index: u32,
    request: &AddRequest,
) -> Result<AddResponse, AppError> {
    let mut guard = lock_editor(state)?;
    let editor = bound(&mut guard)?;

    let transaction = build_transaction(editor.journal(), request)?;
    editor.replace_transaction(Tindex(index), &transaction)?;
    save_and_publish(state, &mut guard)?;

    // An in-place replace adds/removes no transactions and reorders none, so the
    // target keeps its `tindex`.
    let updated = find_transaction(bound(&mut guard)?, index).ok_or_else(|| {
        AppError::Internal("could not locate the replaced transaction after saving".to_string())
    })?;
    Ok(AddResponse {
        index: updated.index.0,
        transaction: native_transaction(updated),
    })
}

fn patch_transaction_locked(
    state: &AppState,
    index: u32,
    request: &PatchRequest,
) -> Result<AddResponse, AppError> {
    let mut guard = lock_editor(state)?;
    let editor = bound(&mut guard)?;

    let changed =
        request.description.is_some() || request.status.is_some() || !request.postings.is_empty();
    if changed {
        // The surgical ops mutate the editor one at a time; if a later one fails
        // after an earlier one committed to memory, re-sync from disk so the
        // in-memory editor and the served snapshot never diverge from the file.
        if let Err(error) = apply_patch(editor, index, request) {
            resync_from_disk(state, &mut guard)?;
            return Err(error.into());
        }
        save_and_publish(state, &mut guard)?;
    }

    let updated =
        find_transaction(bound(&mut guard)?, index).ok_or(EditError::TransactionNotFound(index))?;
    Ok(AddResponse {
        index: updated.index.0,
        transaction: native_transaction(updated),
    })
}

/// The transaction with `tindex` `index` in the editor's current journal.
fn find_transaction(editor: &JournalEditor, index: u32) -> Option<&Transaction> {
    editor
        .journal()
        .transactions
        .iter()
        .find(|txn| txn.index == Tindex(index))
}

/// Apply a [`PatchRequest`]'s surgical edits to `editor` in order (description
/// first, then each posting-account change), stopping at the first error.
fn apply_patch(
    editor: &mut JournalEditor,
    index: u32,
    request: &PatchRequest,
) -> Result<(), EditError> {
    if let Some(description) = &request.description {
        editor.set_description(Tindex(index), description)?;
    }
    if let Some(status) = request.status {
        editor.set_status(Tindex(index), status.into())?;
    }
    for posting in &request.postings {
        editor.set_posting_account(Tindex(index), posting.index, &posting.account)?;
    }
    Ok(())
}

/// Save the editor's pending edit and republish the read snapshot.
///
/// On success the snapshot is rebuilt from the edited journal. On ANY save
/// failure (notably [`EditError::ExternalChange`]) the in-memory edit is
/// unpersisted, so we [`resync_from_disk`] — discarding that edit, re-syncing the
/// rope/fingerprint, and publishing the on-disk state — so the editor and the
/// served snapshot stay consistent with the file. The original save error is
/// then returned (a `409` tells the client to re-fetch/retry), UNLESS the
/// re-sync itself failed, in which case its `500` wins: that is the more serious
/// condition and the one the user has to act on.
fn save_and_publish(state: &AppState, slot: &mut Option<JournalEditor>) -> Result<(), AppError> {
    let editor = bound(slot)?;
    match editor.save() {
        Ok(()) => {
            state.replace_journal(editor.journal());
            Ok(())
        }
        Err(error) => {
            resync_from_disk(state, slot)?;
            Err(error.into())
        }
    }
}

/// Discard any un-saved in-memory edit by re-opening the editor from disk, then
/// republish the snapshot from it — so the editor and the served snapshot both
/// track the on-disk file. Used after a save failure and after a partial
/// (multi-op) edit fails midway.
///
/// DL-5. The re-open MUST be allowed to fail loudly. This used to swallow the
/// error and publish `editor.journal()` regardless — which, when the file had
/// been deleted or made unparseable, published the in-memory journal *still
/// carrying the edit that had just failed to write*. The user got a `409`,
/// re-fetched, and saw their change served back as though it had been saved,
/// while the file on disk had never contained it.
///
/// So on failure we publish NOTHING (the last known-good snapshot stays served)
/// and unbind the editor rather than keep one whose rope holds an unpersisted
/// edit — a later save against it is how that phantom would reach the file.
/// This mirrors [`AppState::reopen_editor`], which unbinds on the same failure.
fn resync_from_disk(state: &AppState, slot: &mut Option<JournalEditor>) -> Result<(), AppError> {
    let Some(path) = slot.as_ref().map(|editor| editor.path().to_path_buf()) else {
        // Nothing bound, so there is no un-saved edit to discard and nothing new
        // to publish; the caller's own error stands.
        return Ok(());
    };
    match JournalEditor::open(path) {
        Ok(reopened) => {
            state.replace_journal(reopened.journal());
            *slot = Some(reopened);
            Ok(())
        }
        Err(error) => {
            *slot = None;
            Err(resync_failed(&error))
        }
    }
}

/// The `500` for DL-5's failure branch: the edit did not reach the file AND the
/// file could not be re-read afterwards, so the server can neither apply nor
/// re-sync. Says both halves plainly, because the dangerous reading of a bare
/// `409` here is "it saved after all".
fn resync_failed(error: &EditError) -> AppError {
    AppError::Internal(format!(
        "your change was NOT saved, and the journal file could not be re-read afterwards to \
         re-sync ({error}). The served data is the last state that was successfully read; \
         editing is disabled until the file is readable again and the journal is re-opened."
    ))
}

/// Lock the editor mutex, recovering from poisoning by re-reading the file.
///
/// SEC-11. A poisoned mutex means some earlier request panicked *while holding
/// the editor*, so the [`JournalEditor`] behind it may be half-mutated: the
/// surgical ops in [`apply_patch`] commit to the in-memory rope one at a time, so
/// a panic between two of them leaves a rope matching no file that ever existed.
/// Taking the inner value and carrying on — what this used to do — edits the
/// user's journal from that state.
///
/// Recovery is therefore the same one [`resync_from_disk`] already performs after
/// a failed save: drop the suspect editor and re-open from disk. The poison flag
/// is cleared only once the editor behind it is trustworthy again, and only while
/// still holding the guard, so no other thread can observe the un-recovered
/// state. If the re-open FAILS we unbind the editor rather than hand back a
/// half-mutated one, and the caller gets a 500 — never a silent edit against
/// corrupt state.
fn lock_editor(state: &AppState) -> Result<MutexGuard<'_, Option<JournalEditor>>, AppError> {
    let mutex = state.editor();
    let mut guard = match mutex.lock() {
        Ok(guard) => return Ok(guard),
        Err(poisoned) => poisoned.into_inner(),
    };
    let reopened = guard
        .as_ref()
        .map(|editor| JournalEditor::open(editor.path().to_path_buf()));
    match reopened {
        // No editor was bound, so there is nothing half-mutated to discard; the
        // caller's own `editing_disabled` 501 remains the right answer.
        None => {}
        Some(Ok(editor)) => {
            state.replace_journal(editor.journal());
            *guard = Some(editor);
        }
        Some(Err(_)) => {
            *guard = None;
            mutex.clear_poison();
            return Err(editor_poisoned());
        }
    }
    mutex.clear_poison();
    Ok(guard)
}

/// The `500` returned when a prior panic poisoned the editor and the file could
/// not be re-read to recover a trustworthy one (SEC-11).
fn editor_poisoned() -> AppError {
    AppError::Internal(
        "the editor was left in an indeterminate state by an earlier failure and the journal file \
         could not be re-read to recover; no edit was applied"
            .to_string(),
    )
}

/// The 0-based file-order position the new transaction will occupy after the
/// reparse — mirrors [`JournalEditor`]'s placement so we can fetch the added row
/// back out. `Append` lands at the end. `DateOrdered` lands right after its
/// predecessor (the latest transaction dated `<=` the new one, last such in file
/// order ⇒ `predecessor + 1`); with no predecessor it lands AT the earliest
/// transaction's position (inserted just before it); an empty journal lands at 0.
fn insertion_index(journal: &Journal, txn: &Transaction, position: InsertPosition) -> usize {
    let transactions = &journal.transactions;
    let len = transactions.len();
    match position {
        InsertPosition::Append => len,
        InsertPosition::DateOrdered => transactions
            .iter()
            .enumerate()
            .filter(|(_, existing)| existing.date.as_str() <= txn.date.as_str())
            .max_by(|(_, a), (_, b)| a.date.cmp(&b.date))
            .map_or_else(
                || {
                    transactions
                        .iter()
                        .enumerate()
                        .min_by(|(_, a), (_, b)| a.date.cmp(&b.date))
                        .map_or(len, |(earliest, _)| earliest)
                },
                |(predecessor, _)| predecessor + 1,
            ),
    }
}

// ===========================================================================
// Build a `core::Transaction` from the request (with inferred styles)
// ===========================================================================

fn build_transaction(journal: &Journal, request: &AddRequest) -> Result<Transaction, AppError> {
    if request.date.trim().is_empty() {
        return Err(AppError::BadRequest(
            "a transaction needs a date".to_string(),
        ));
    }
    if request.postings.is_empty() {
        return Err(AppError::BadRequest(
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
        // A blank/whitespace `date2` means "no secondary date"; otherwise store the
        // trimmed value so `format_header` emits `DATE=DATE2` cleanly.
        date2: request
            .date2
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned),
        status: request.status.map_or(Status::Unmarked, Status::from),
        code: request.code.clone().unwrap_or_default(),
        description: request.description.clone().unwrap_or_default(),
        comment: comment_string(request.comment.as_deref()),
        preceding_comment: String::new(),
        tags: Vec::new(),
        postings,
        // Placeholder; recomputed on reparse.
        source_span: (
            SourcePos { line: 1, column: 1 },
            SourcePos { line: 1, column: 1 },
        ),
        // Placeholder; the editor assigns the file the transaction lands in.
        source_file: std::path::PathBuf::new(),
    })
}

fn build_posting(journal: &Journal, input: &PostingIn) -> Result<Posting, AppError> {
    if input.account.trim().is_empty() {
        return Err(AppError::BadRequest(
            "a posting needs an account".to_string(),
        ));
    }
    let amounts = match &input.amount {
        Some(amount) => vec![build_amount(journal, amount)?],
        None => Vec::new(),
    };
    let balance_assertion = input
        .balance_assertion
        .as_ref()
        .map(|assertion| build_assertion(journal, assertion, &input.account, amounts.is_empty()))
        .transpose()?;
    Ok(Posting {
        status: input.status.map_or(Status::Unmarked, Status::from),
        ptype: input.ptype.map_or(PostingType::Regular, PostingType::from),
        account: AccountName(input.account.clone()),
        amounts,
        balance_assertion,
        date: None,
        date2: None,
        comment: comment_string(input.comment.as_deref()),
        tags: Vec::new(),
    })
}

/// Build a [`BalanceAssertion`] from the wire, refusing the two shapes the
/// journal must never be asked to hold (DL-2's "do not let a malformed
/// assertion reach the core").
///
/// * **A blank commodity** would render as a BARE asserted number (`= 99.00`),
///   which re-reads under a journal `D` default-commodity directive as a
///   *different* commodity. That is a silent change of which balance is being
///   anchored, and the round-trip guard cannot see it as one because the text it
///   wrote is the text it reads back.
/// * **An assertion with no amount on the same posting.** hledger accepts
///   `assets:cash  = $99.00`, but this writer cannot produce it: the formatter
///   emits an assertion only alongside a posting's first amount line, so an
///   amount-less posting would drop it on the floor — the exact silent loss
///   DL-2 is about. Refusing is the honest answer; the SPA always sends the
///   balanced amount, so this is unreachable from the popup.
fn build_assertion(
    journal: &Journal,
    input: &BalanceAssertionIn,
    account: &str,
    posting_has_no_amount: bool,
) -> Result<BalanceAssertion, AppError> {
    if input.amount.commodity.trim().is_empty() {
        return Err(AppError::BadRequest(format!(
            "the balance assertion on '{account}' needs a commodity: a bare asserted number \
             would re-read as whatever commodity the journal defaults to"
        )));
    }
    if posting_has_no_amount {
        return Err(AppError::BadRequest(format!(
            "the balance assertion on '{account}' needs an amount on the same posting: an \
             assertion cannot be written on the inferred (elided) leg"
        )));
    }
    Ok(BalanceAssertion {
        amount: build_priced_amount(journal, &input.amount)?,
        inclusive: input.inclusive,
        total: input.total,
        // Placeholder; the reparse recomputes it, like the transaction's own span.
        position: SourcePos { line: 1, column: 1 },
    })
}

/// Normalize an optional wire comment into the model's stored form: the trimmed
/// text with a single trailing newline (matching the parser's `build_comment`),
/// or empty when absent or blank. The formatter re-adds the `  ; ` prefix.
fn comment_string(raw: Option<&str>) -> String {
    match raw {
        Some(text) if !text.trim().is_empty() => format!("{}\n", text.trim()),
        _ => String::new(),
    }
}

fn build_amount(journal: &Journal, input: &AmountIn) -> Result<Amount, AppError> {
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

fn build_cost(journal: &Journal, input: &CostIn) -> Result<Cost, AppError> {
    Ok(Cost {
        kind: input.kind.into(),
        amount: build_priced_amount(journal, &input.amount)?,
    })
}

/// A bare (cost-free) [`Amount`] from the wire, with its style inferred from the
/// journal. Shared by the cost annotation and the balance assertion, which are
/// the two places an amount can appear without a nested cost of its own.
fn build_priced_amount(journal: &Journal, input: &PricedAmountIn) -> Result<Amount, AppError> {
    let commodity = Commodity(input.commodity.clone());
    let quantity = dec_from_wire(&input.quantity)?;
    let style = infer_style(journal, &commodity, quantity.places);
    Ok(Amount {
        commodity,
        quantity,
        style,
        cost: None,
    })
}

/// The largest `|mantissa|` the edit wire accepts: 10^30, i.e. a 31-digit
/// significand. `Dec` math is `i128` (max ~1.7×10^38), so this leaves eight
/// decimal orders of headroom for the summing and price-scaling every report
/// does, while still being astronomically larger than any real financial amount
/// (a nonillion units, at ten decimal places). A value above this is a
/// malformed or hostile client, not a transaction someone means to record.
const MAX_WIRE_MANTISSA: i128 = 10_i128.pow(30);

/// Convert a wire decimal to a [`Dec`], rejecting values the journal must never
/// be asked to hold.
///
/// Both bounds are SEC-5. Neither the reparse-to-validate nor the round-trip
/// guard downstream catches an absurd-but-self-consistent amount: they check
/// that the value the client asked for is the value that landed, not that it was
/// a sane thing to ask for. `places` is bounded by [`MAX_PARSE_PLACES`] — the
/// precision the PARSER stores — so the wire cannot introduce an amount that
/// re-reading the journal could never reproduce. Without it,
/// `{"mantissa":"0","places":65534}` was accepted with a `201` and committed a
/// multi-hundred-byte all-zeros amount line to the user's books.
pub(crate) fn dec_from_wire(dec: &WireDecIn) -> Result<Dec, AppError> {
    let mantissa = dec.mantissa.trim().parse::<i128>().map_err(|_| {
        AppError::BadRequest(format!(
            "invalid amount mantissa '{}': expected a base-10 integer string",
            dec.mantissa
        ))
    })?;
    if dec.places > MAX_PARSE_PLACES {
        return Err(AppError::BadRequest(format!(
            "amount places {} is out of range (expected 0..={MAX_PARSE_PLACES})",
            dec.places
        )));
    }
    if mantissa.unsigned_abs() > MAX_WIRE_MANTISSA.unsigned_abs() {
        return Err(AppError::BadRequest(format!(
            "amount mantissa '{}' is out of range (expected |mantissa| <= {MAX_WIRE_MANTISSA})",
            dec.mantissa
        )));
    }
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
pub(crate) fn infer_style(journal: &Journal, commodity: &Commodity, places: u32) -> AmountStyle {
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
        balance_assertion: posting.balance_assertion.as_ref().map(|assertion| {
            NativeBalanceAssertion {
                amount: native_amount(&assertion.amount),
                inclusive: assertion.inclusive,
                total: assertion.total,
            }
        }),
    }
}

fn native_amount(amount: &Amount) -> NativeAmount {
    NativeAmount {
        commodity: amount.commodity.0.clone(),
        quantity: amount.quantity.into(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic::AssertUnwindSafe;

    const ONE_TXN: &str = "\
2024-01-01 * A
    expenses:a  $1.00
    assets:bank
";

    /// Panic while holding the editor guard, exactly as a mid-edit panic would.
    fn poison_editor(state: &AppState) {
        let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let _guard = state.editor().lock().expect("not yet poisoned");
            panic!("simulated mid-edit panic");
        }));
        assert!(result.is_err(), "the closure must have panicked");
        assert!(state.editor().is_poisoned(), "the mutex must be poisoned");
    }

    /// SEC-11: after a panic left the mutex poisoned, `lock_editor` must NOT hand
    /// back the possibly half-mutated editor. It re-opens from disk — so the
    /// editor is once again a faithful view of the file — and clears the poison
    /// so later requests take the normal path.
    #[test]
    fn poisoned_editor_is_recovered_by_reopening_from_disk() {
        let dir = std::env::temp_dir().join("ledgeline-poison-tests");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join(format!("poison-{}.journal", std::process::id()));
        std::fs::write(&path, ONE_TXN).expect("write journal");

        let state = AppState::from_journal_path(&path).expect("editor opens");
        poison_editor(&state);

        // Recovery succeeds, yields a usable editor, and clears the poison.
        let guard = lock_editor(&state).expect("poisoned lock must recover");
        assert!(guard.is_some(), "an editor must still be bound");
        assert_eq!(guard.as_ref().unwrap().journal().transactions.len(), 1);
        drop(guard);
        assert!(
            !state.editor().is_poisoned(),
            "the poison flag must be cleared once the editor is trustworthy again"
        );

        // The normal (unpoisoned) path still works afterwards.
        assert!(lock_editor(&state).is_ok());
        let _ = std::fs::remove_file(&path);
    }

    /// SEC-11, the failure branch: if the file cannot be re-read there is no
    /// trustworthy editor to hand back, so the request must fail with a 500 and
    /// the editor must be UNBOUND rather than left half-mutated.
    #[test]
    fn poisoned_editor_that_cannot_reopen_is_unbound_and_errors() {
        let dir = std::env::temp_dir().join("ledgeline-poison-tests");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join(format!("poison-gone-{}.journal", std::process::id()));
        std::fs::write(&path, ONE_TXN).expect("write journal");

        let state = AppState::from_journal_path(&path).expect("editor opens");
        poison_editor(&state);
        // The file disappears before recovery can re-read it.
        std::fs::remove_file(&path).expect("remove journal");

        match lock_editor(&state) {
            Ok(_) => panic!("recovery must fail once the journal file is gone"),
            Err(error) => assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR),
        }

        // The suspect editor was dropped rather than served.
        let guard = lock_editor(&state).expect("no longer poisoned");
        assert!(
            guard.is_none(),
            "the un-recoverable editor must have been unbound, not kept"
        );
    }
}
