//! The invoicer API under `/api` (ARCHITECTURE §6.2). Every route needs
//! the bearer token ([`ledgeline_shell::security::protect`], applied by the
//! caller), and every body is marked `no-store`.
//!
//! Phase 3a serves what the shell needs plus the read-only views the CLI
//! already has, in the CLI's `--json` shapes (`invoicer_core::views`):
//!
//! | Route                       | Answers                                        |
//! |-----------------------------|------------------------------------------------|
//! | `GET /api/version`          | The app and its version.                       |
//! | `GET /api/books`            | The open books: journal, basis, counts.        |
//! | `GET /api/invoices`         | `list --json` (`status`, `customer`, `type`, `open`, `overdue`, `today`). |
//! | `GET /api/invoices/{num}`   | `show --json` (`today`).                       |
//! | `GET /api/invoices/{num}/books` | The Books panel: what the journal holds (3b). |
//! | `POST /api/preview`         | An invoice rendered from unsaved text (3b).    |
//! | `GET /api/templates`        | The templates, and starters not copied in (3b). |
//! | `GET /api/invoices/{num}/pdf` | Sending it: where a PDF goes, kept PDFs, the portal, `sent_hash` (3c). |
//! | `POST /api/invoices/{num}/pdf` | Export its PDF ([`super::pdf`], 3c).         |
//! | `POST /api/invoices/{num}/send` | Mark it sent (book it), `sent_via` (3c).    |
//! | `POST /api/invoices/{num}/portal` | Open its customer's billing portal (3c).  |
//! | `POST /api/invoices/{num}/template` | Use a template: write `template:`, copy a starter in ([`super::edits`]). |
//! | `POST /api/invoices/{num}/recalc` | Rewrite its amounts; a sent one is re-booked too ([`super::edits`]). |
//! | `POST /api/pdf/share`, `/api/pdf/copy` | Share… or Copy file an exported PDF (3c). |
//! | `POST /api/pdf/open`        | Open an exported PDF in the default viewer.    |
//! | `GET /api/events`           | Change events (server-sent events), see [`events`]. |
//! | `GET /api/customers`        | `customers list --json`.                       |
//! | `GET`/`PATCH /api/ui-prefs` | The shared theme and scale (Ledgeline's shape). |
//! | `POST /api/session`         | The cookie MBR's pages need ([`super::mbr`]).  |
//!
//! Errors are `{"error": "…"}` with a status: `400` for a bad parameter,
//! `404` for an unknown invoice, `409` when no books are open, `422` for a
//! page template that can't render (a fixable problem in the files).

use std::sync::Arc;

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use invoicer_core::doc::{DocType, Status};
use invoicer_core::index::ListFilter;
use invoicer_core::table::books::is_iso_date;
use invoicer_core::views::{self, ListQuery};
use ledgeline_core::ui_prefs::{Patch, UiPrefsError};
use ledgeline_shell::security::no_store_json;
use serde::Deserialize;
use serde_json::{Value, json};

use super::books::OpenBooks;
use super::edits;
use super::mbr::MbrGate;
use super::pdf;
use super::preview::{self, PreviewError, PreviewRequest};
use super::{AppState, prefs};

/// The API routes (state still to be supplied).
pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/version", get(version))
        .route("/api/books", get(books))
        .route("/api/invoices", get(invoices))
        .route("/api/invoices/{number}", get(invoice))
        .route("/api/invoices/{number}/books", get(invoice_books))
        .route("/api/preview", post(preview))
        .route("/api/templates", get(templates))
        .route(
            "/api/invoices/{number}/pdf",
            get(pdf::delivery).post(pdf::export),
        )
        .route("/api/invoices/{number}/send", post(pdf::send))
        .route("/api/invoices/{number}/portal", post(pdf::open_portal))
        .route("/api/invoices/{number}/template", post(edits::use_template))
        .route("/api/invoices/{number}/recalc", post(edits::recalc))
        .route("/api/pdf/share", post(pdf::share))
        .route("/api/pdf/copy", post(pdf::copy_file))
        .route("/api/pdf/open", post(pdf::open_pdf))
        .route("/api/events", get(events))
        .route("/api/customers", get(customers))
        .route("/api/ui-prefs", get(ui_prefs).patch(patch_ui_prefs))
        .route("/api/session", post(session))
        // The invoice editor (4a): its own table, merged.
        .merge(super::editor::routes())
}

/// An API failure: a status and a sentence.
#[derive(Debug)]
pub(crate) struct ApiError(pub(crate) StatusCode, pub(crate) String);

impl ApiError {
    pub(crate) fn bad_request(message: impl Into<String>) -> Self {
        Self(StatusCode::BAD_REQUEST, message.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut response = no_store_json(json!({ "error": self.1 }));
        *response.status_mut() = self.0;
        response
    }
}

impl From<UiPrefsError> for ApiError {
    fn from(error: UiPrefsError) -> Self {
        match error {
            UiPrefsError::BadRequest(m) => Self(StatusCode::BAD_REQUEST, m),
            UiPrefsError::Conflict(m) => Self(StatusCode::CONFLICT, m),
            UiPrefsError::Internal(m) => Self(StatusCode::INTERNAL_SERVER_ERROR, m),
        }
    }
}

pub(crate) type ApiResult = Result<Response, ApiError>;

/// Run `f` over the open books on the blocking pool (it locks the session
/// and may read files), answering its JSON.
pub(crate) async fn with_books<F>(state: &AppState, f: F) -> ApiResult
where
    F: FnOnce(&OpenBooks) -> Result<Value, ApiError> + Send + 'static,
{
    let books = state.books().ok_or_else(no_books)?;
    blocking(move || f(&books)).await
}

pub(crate) fn no_books() -> ApiError {
    ApiError(
        StatusCode::CONFLICT,
        "no books are open; choose File → Open".to_string(),
    )
}

/// Run `f` on the blocking pool, answering its JSON.
pub(crate) async fn blocking<F>(f: F) -> ApiResult
where
    F: FnOnce() -> Result<Value, ApiError> + Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(result) => result.map(no_store_json),
        Err(error) => Err(ApiError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("the request failed: {error}"),
        )),
    }
}

pub(crate) fn to_value(value: impl serde::Serialize) -> Result<Value, ApiError> {
    serde_json::to_value(value)
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

/// `?today=` (YYYY-MM-DD), else the local date (as the CLI, PLAN C5).
pub(crate) fn today(given: Option<&str>) -> Result<String, ApiError> {
    match given {
        Some(date) if is_iso_date(date) => Ok(date.to_string()),
        Some(date) => Err(ApiError::bad_request(format!(
            "today={date}: not a date (YYYY-MM-DD)"
        ))),
        None => Ok(chrono::Local::now().format("%Y-%m-%d").to_string()),
    }
}

async fn version() -> Response {
    no_store_json(json!({
        "app": "ledgeline-invoicer",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// `GET /api/books`: the journal's file name (never its full path), the
/// basis, and the counts the shell shows.
async fn books(State(state): State<AppState>) -> ApiResult {
    with_books(&state, |books| {
        let session = books.session();
        let next = session
            .next_number()
            .map(|n| n.to_string())
            .map_err(|e| ApiError(StatusCode::CONFLICT, e.to_string()))?;
        Ok(json!({
            "journal": books.journal().file_name().map(|n| n.to_string_lossy().into_owned()),
            "basis": to_value(session.basis())?,
            "documents": session.index().docs().count(),
            "customers": session.index().customers().len(),
            "nextNumber": next,
        }))
    })
    .await
}

#[derive(Debug, Default, Deserialize)]
struct ListParams {
    status: Option<String>,
    customer: Option<String>,
    #[serde(rename = "type")]
    doc_type: Option<String>,
    #[serde(default)]
    open: bool,
    #[serde(default)]
    overdue: bool,
    today: Option<String>,
}

/// `GET /api/invoices`: `list --json`.
async fn invoices(State(state): State<AppState>, Query(params): Query<ListParams>) -> ApiResult {
    let today = today(params.today.as_deref())?;
    let query = ListQuery {
        filter: ListFilter {
            doc_type: params
                .doc_type
                .as_deref()
                .map(|t| DocType::parse(t).ok_or_else(|| bad("type", t)))
                .transpose()?,
            status: params
                .status
                .as_deref()
                .map(|s| Status::parse(s).ok_or_else(|| bad("status", s)))
                .transpose()?,
            customer: params.customer,
        },
        open: params.open,
        overdue: params.overdue,
    };
    with_books(&state, move |books| {
        let session = books.session();
        let problems = views::document_problems(&session);
        to_value(views::list(
            session.index(),
            Some(session.book_states()),
            &problems,
            &query,
            &today,
        ))
    })
    .await
}

fn bad(what: &str, value: &str) -> ApiError {
    ApiError::bad_request(format!("{what}={value}: not a {what}"))
}

#[derive(Debug, Default, Deserialize)]
struct TodayParam {
    today: Option<String>,
}

/// `GET /api/invoices/{number}`: `show --json`, by number or path.
async fn invoice(
    State(state): State<AppState>,
    Path(number): Path<String>,
    Query(params): Query<TodayParam>,
) -> ApiResult {
    let today = today(params.today.as_deref())?;
    with_books(&state, move |books| {
        let session = books.session();
        let doc = session
            .find(&number)
            .map_err(|e| ApiError(StatusCode::NOT_FOUND, e.to_string()))?;
        let mut problems = views::document_problems(&session);
        if let Some(number) = doc.number() {
            problems.extend(views::books_problems(&session, Some(&[number.to_string()])));
        }
        let detail = views::show(&session, doc, &problems, &today)
            .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        to_value(detail)
    })
    .await
}

/// `GET /api/invoices/{number}/books`: the Books panel (what the journal
/// holds under the invoice's number), by number or path.
async fn invoice_books(State(state): State<AppState>, Path(number): Path<String>) -> ApiResult {
    with_books(&state, move |books| {
        let session = books.session();
        let doc = session
            .find(&number)
            .map_err(|e| ApiError(StatusCode::NOT_FOUND, e.to_string()))?;
        to_value(views::invoice_books(&session, doc))
    })
    .await
}

impl From<PreviewError> for ApiError {
    fn from(error: PreviewError) -> Self {
        match error {
            PreviewError::BadRequest(m) => Self(StatusCode::BAD_REQUEST, m),
            PreviewError::NotFound(m) => Self(StatusCode::NOT_FOUND, m),
            PreviewError::Template(m) => Self(StatusCode::UNPROCESSABLE_ENTITY, m),
            PreviewError::Internal(m) => Self(StatusCode::INTERNAL_SERVER_ERROR, m),
        }
    }
}

/// `POST /api/preview`: render an invoice (unsaved text, or the saved file)
/// through its template or another ([`preview`]). Writes nothing.
async fn preview(
    State(state): State<AppState>,
    payload: Result<Json<PreviewRequest>, JsonRejection>,
) -> ApiResult {
    let Json(request) = payload.map_err(|e| ApiError::bad_request(e.body_text()))?;
    let books = state.books().ok_or_else(no_books)?;
    let rendered = tokio::task::spawn_blocking(move || preview::render(&books, request))
        .await
        .map_err(|e| {
            ApiError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("the request failed: {e}"),
            )
        })??;
    let mut response = no_store_json(to_value(&rendered)?);
    if let Ok(timing) =
        axum::http::HeaderValue::from_str(&format!("render;dur={:.2}", rendered.render_ms))
    {
        response.headers_mut().insert("server-timing", timing);
    }
    Ok(response)
}

/// `GET /api/templates`: the books' templates, then the starters not
/// copied in yet.
async fn templates(State(state): State<AppState>) -> ApiResult {
    with_books(&state, |books| to_value(preview::templates(books))).await
}

/// `GET /api/events`: a stream of server-sent events, so the SPA shows a
/// change made outside it (an editor, a bank import) within about a second.
///
/// - `ready` (`{}`), first: the stream is live; a reconnecting SPA reloads
///   what it shows, since it may have missed changes meanwhile;
/// - `changed` (`{"paths": [...]}`): files changed on disk and the books were
///   re-read and reconciled ([`super::books::Changed`]); `paths` is empty
///   when this listener fell behind and missed some;
/// - a comment every 15 s keeps it open.
///
/// Token-gated like the rest of `/api`, so the SPA reads it with `fetch`
/// (an `EventSource` can't send the header). It ends when the books are
/// swapped (File → Open), and the SPA reconnects. `409` with no books.
async fn events(State(state): State<AppState>) -> Result<Response, ApiError> {
    use axum::response::sse::{Event, KeepAlive, Sse};
    use futures_util::stream::{self, StreamExt as _};
    use tokio::sync::broadcast::error::RecvError;

    let books = state.books().ok_or_else(no_books)?;
    let receiver = books.subscribe();
    // The stream must not keep the books alive past File → Open.
    drop(books);
    let ready = stream::once(async {
        Ok::<_, std::convert::Infallible>(Event::default().event("ready").data("{}"))
    });
    let changes = stream::unfold(receiver, |mut receiver| async move {
        let changed = match receiver.recv().await {
            Ok(changed) => changed,
            Err(RecvError::Lagged(_)) => super::books::Changed { paths: Vec::new() },
            Err(RecvError::Closed) => return None,
        };
        let data = serde_json::to_string(&changed).unwrap_or_else(|_| "{\"paths\":[]}".to_string());
        Some((Ok(Event::default().event("changed").data(data)), receiver))
    });
    let mut response = Sse::new(ready.chain(changes))
        .keep_alive(KeepAlive::default())
        .into_response();
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        ledgeline_shell::security::no_store(),
    );
    Ok(response)
}

/// `GET /api/customers`: `customers list --json`.
async fn customers(State(state): State<AppState>) -> ApiResult {
    with_books(&state, |books| {
        let session = books.session();
        to_value(views::customers(
            session.index(),
            Some(session.book_states()),
        ))
    })
    .await
}

/// `GET /api/ui-prefs`: the effective theme and scale. Reads only.
async fn ui_prefs(State(state): State<AppState>) -> ApiResult {
    blocking(move || to_value(prefs::current(&state))).await
}

/// `PATCH /api/ui-prefs`: save the named keys, as Ledgeline does (in the
/// books' `ledgeline.toml` when it already sets that key, else in the
/// shared `ui-prefs.toml`).
async fn patch_ui_prefs(
    State(state): State<AppState>,
    payload: Result<Json<Value>, JsonRejection>,
) -> ApiResult {
    let Json(body) = payload.map_err(|e| ApiError::bad_request(e.body_text()))?;
    let patch = Patch::from_json(&prefs::SCHEMA, &body)?;
    blocking(move || to_value(prefs::apply(&state, &patch)?)).await
}

/// `POST /api/session`: the cookie MBR's pages need.
async fn session(Extension(gate): Extension<Arc<MbrGate>>) -> Response {
    gate.session_response()
}
