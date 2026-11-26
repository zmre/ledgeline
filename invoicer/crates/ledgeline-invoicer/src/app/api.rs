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
//! | `GET /api/dashboard`, `POST /api/ledgeline/open` | The receivables dashboard; the books in Ledgeline ([`super::dashboard`], 5a). |
//! | `GET /api/invoices`         | `list --json` (`status`, `customer`, `type`, `open`, `overdue`, `from`, `to`, `aged`, `q`, `sort`, `dir`, `today`; PLAN 5b S1). |
//! | `GET /api/invoices/{num}`   | `show --json` (`today`).                       |
//! | `GET /api/invoices/{num}/books` | The Books panel: what the journal holds (3b). |
//! | `POST /api/preview`         | An invoice rendered from unsaved text (3b).    |
//! | `/api/templates…`           | The templates, and starters not copied in (3b); the template editor (4d): [`super::templates`]. |
//! | `GET /api/invoices/{num}/pdf` | Sending it: where a PDF goes, kept PDFs, the portal, `sent_hash` (3c). |
//! | `POST /api/invoices/{num}/pdf` | Export its PDF ([`super::pdf`], 3c).         |
//! | `POST /api/invoices/{num}/send` | Mark it sent (book it), `sent_via` (3c).    |
//! | `POST /api/invoices/{num}/portal` | Open its customer's billing portal (3c).  |
//! | `POST /api/invoices/{num}/template` | Use a template: write `template:`, copy a starter in ([`super::edits`]). |
//! | `POST /api/invoices/{num}/recalc` | Rewrite its amounts; a sent one is re-booked too ([`super::edits`]). |
//! | `POST /api/pdf/share`, `/api/pdf/copy` | Share… or Copy file an exported PDF (3c). |
//! | `POST /api/pdf/open`        | Open an exported PDF in the default viewer.    |
//! | `GET /api/events`           | Change events (server-sent events), see [`events`]. |
//! | `GET`/`POST /api/customers`, `GET`/`PUT /api/customers/{*key}` | Customers: list, create, show, edit ([`super::customers`], 4b). |
//! | `GET`/`POST /api/catalog`, `PUT /api/catalog/{*path}`; `GET`/`POST /api/snippets`, `PUT`/`DELETE /api/snippets/{*path}` | The catalog and snippets editors ([`super::library`], 4c). |
//! | `GET /api/files`, `POST /api/files/pick`, `/api/files/open` | Files in the books folder for contracts: list, the open dialog, open one ([`super::files`]). |
//! | `GET`/`PATCH /api/ui-prefs`, `POST /api/ui-prefs/pin` | The shared theme, scale and payment matching, and keeping one for these books (Ledgeline's shapes). |
//! | `GET`/`PUT /api/settings`, `POST /api/settings/check` | The Settings page: `[invoicer]`, read and saved ([`super::settings`]). |
//! | `POST /api/session`         | The cookie MBR's pages need ([`super::mbr`]).  |
//! | the invoice editor's routes | Form, compute, save, new, void, rebook… (4a): [`super::editor`]. |
//!
//! Every route answers errors in one envelope ([`ApiError`]):
//! `{"error": "…", "code": "…"}` with a status, plus any detail the case
//! has (`revision`, `problems`, …). A route that names no case of its own
//! answers the status's code: `bad-request` (400, a bad parameter),
//! `not-found` (404, an unknown invoice), `conflict` (409, e.g. no books
//! open), `invalid` (422, a fixable problem in the files, such as a page
//! template that can't render), `unavailable` (503), `internal` (500 and
//! the rest). The editor's, library's and templates' routes
//! ([`super::editor`], [`super::library`], [`super::templates`]) name more
//! cases; see their module docs. The books bridge's errors carry
//! [`invoicer_core::books::BooksError::code`] ([`super::pdf::books_error`]).

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
use ledgeline_core::ui_prefs::{Patch, Pin, UiPrefsError};
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
        .merge(super::customers::routes())
        .merge(super::dashboard::routes())
        .merge(super::library::routes())
        .merge(super::templates::routes())
        .merge(super::files::routes())
        .route("/api/ui-prefs", get(ui_prefs).patch(patch_ui_prefs))
        .route("/api/ui-prefs/pin", post(pin_ui_pref))
        .merge(super::settings::routes())
        .route("/api/session", post(session))
        // The invoice editor (4a): its own table, merged.
        .merge(super::editor::routes())
}

/// An API failure, the one envelope every route under `/api` answers with
/// (RA-4): `{"error": "…", "code": "…"}` plus any `detail` (`revision`,
/// `problems`, `customers`, …) merged in, with its status, `no-store`.
///
/// `code` is the stable name the SPA branches on. A route that names no
/// case gets the status's ([`code_for`]), so the same status always comes
/// back with the same code unless the route says more.
#[derive(Debug)]
pub(crate) struct ApiError {
    pub(crate) status: StatusCode,
    pub(crate) code: &'static str,
    pub(crate) message: String,
    detail: Vec<(&'static str, Value)>,
}

/// The code a status answers with when the route names no case of its own.
pub(crate) fn code_for(status: StatusCode) -> &'static str {
    match status {
        StatusCode::BAD_REQUEST => "bad-request",
        StatusCode::NOT_FOUND => "not-found",
        StatusCode::CONFLICT => "conflict",
        StatusCode::UNPROCESSABLE_ENTITY => "invalid",
        StatusCode::SERVICE_UNAVAILABLE => "unavailable",
        _ => "internal",
    }
}

impl ApiError {
    /// A failure with its own code.
    pub(crate) fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            detail: Vec::new(),
        }
    }

    /// A failure with the status's code ([`code_for`]).
    pub(crate) fn status(status: StatusCode, message: impl Into<String>) -> Self {
        Self::new(status, code_for(status), message)
    }

    pub(crate) fn bad_request(message: impl Into<String>) -> Self {
        Self::status(StatusCode::BAD_REQUEST, message)
    }

    pub(crate) fn not_found(message: impl Into<String>) -> Self {
        Self::status(StatusCode::NOT_FOUND, message)
    }

    pub(crate) fn internal(message: impl Into<String>) -> Self {
        Self::status(StatusCode::INTERNAL_SERVER_ERROR, message)
    }

    /// `key: value` merged into the body (`revision`, `problems`, …).
    pub(crate) fn with(mut self, key: &'static str, value: Value) -> Self {
        self.detail.push((key, value));
        self
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut body = json!({ "error": self.message, "code": self.code });
        for (key, value) in self.detail {
            body[key] = value;
        }
        let mut response = no_store_json(body);
        *response.status_mut() = self.status;
        response
    }
}

impl From<UiPrefsError> for ApiError {
    fn from(error: UiPrefsError) -> Self {
        match error {
            UiPrefsError::BadRequest(m) => Self::status(StatusCode::BAD_REQUEST, m),
            UiPrefsError::Conflict(m) => Self::status(StatusCode::CONFLICT, m),
            UiPrefsError::Internal(m) => Self::internal(m),
        }
    }
}

impl From<PreviewError> for ApiError {
    fn from(error: PreviewError) -> Self {
        let (status, message) = error.status_and_message();
        Self::status(status, message)
    }
}

pub(crate) type ApiResult = Result<Response, ApiError>;

/// Run `f` over the open books on the blocking pool (it locks the session
/// and may read files), answering its JSON with `status`: the one helper
/// every route that reads the books uses. `409` when no books are open.
pub(crate) async fn with_books<F>(state: &AppState, status: StatusCode, f: F) -> ApiResult
where
    F: FnOnce(&OpenBooks) -> Result<Value, ApiError> + Send + 'static,
{
    let books = state.open_books()?;
    blocking(status, move || f(&books)).await
}

/// `409`: no books are open. When the books were asked for but couldn't
/// open (a problem in `ledgeline.toml`, say), `problem` says why, so every
/// page can, and `GET /api/settings` still answers.
pub(crate) fn no_books(unopened: Option<&super::Unopened>) -> ApiError {
    match unopened {
        Some(unopened) => ApiError::status(
            StatusCode::CONFLICT,
            format!("the books couldn't open: {}", unopened.problem),
        )
        .with("problem", Value::String(unopened.problem.clone())),
        None => ApiError::status(
            StatusCode::CONFLICT,
            "no books are open; choose File → Open",
        ),
    }
}

/// Run `f` on the blocking pool, answering its JSON with `status`.
pub(crate) async fn blocking<F>(status: StatusCode, f: F) -> ApiResult
where
    F: FnOnce() -> Result<Value, ApiError> + Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(result) => result.map(|value| {
            let mut response = no_store_json(value);
            *response.status_mut() = status;
            response
        }),
        Err(error) => Err(ApiError::internal(format!("the request failed: {error}"))),
    }
}

/// A request body, or `400` saying why it doesn't read.
pub(crate) fn body<T>(payload: Result<Json<T>, JsonRejection>) -> Result<T, ApiError> {
    payload
        .map(|Json(body)| body)
        .map_err(|e| ApiError::bad_request(e.body_text()))
}

pub(crate) fn to_value(value: impl serde::Serialize) -> Result<Value, ApiError> {
    serde_json::to_value(value).map_err(|e| ApiError::internal(e.to_string()))
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

/// `GET /api/books`'s query.
#[derive(Debug, Default, Deserialize)]
struct BooksParams {
    /// For "overdue" (YYYY-MM-DD); the local date by default.
    today: Option<String>,
}

/// `GET /api/books`: the journal's file name (never its full path), the
/// books' name as Ledgeline shows it (`ledgeline_core::title`: the main
/// file's leading comment, else its folder's name), the basis, and the
/// counts the sidebar shows: invoices (all, drafts, open, overdue on
/// `today`), customers, catalog items, snippets and templates; and the next number
/// with what it follows (`nextNumberAfter`, `nextNumberSource`).
async fn books(State(state): State<AppState>, Query(params): Query<BooksParams>) -> ApiResult {
    let today = today(params.today.as_deref())?;
    with_books(&state, StatusCode::OK, move |books| {
        let session = books.session();
        let next = session
            .explain_next_number()
            .map_err(|e| ApiError::status(StatusCode::CONFLICT, e.to_string()))?;
        let index = session.index();
        let states = Some(session.book_states());
        let invoices: Vec<_> = index
            .docs()
            .filter(|d| d.header.doc_type.unwrap_or(DocType::Invoice) == DocType::Invoice)
            .collect();
        let catalog_items: usize = index.catalogs().map(|c| c.items.len()).sum();
        Ok(json!({
            "journal": books.journal().file_name().map(|n| n.to_string_lossy().into_owned()),
            "title": ledgeline_core::title::journal_title(session.journal()),
            "basis": to_value(session.basis())?,
            "documents": index.docs().count(),
            "invoices": {
                "all": invoices.len(),
                "drafts": invoices.iter().filter(|d| d.header.status == Some(Status::Draft)).count(),
                "open": invoices.iter().filter(|d| views::open_in(states, d)).count(),
                "overdue": invoices.iter().filter(|d| views::overdue_in(states, d, &today)).count(),
            },
            "customers": index.customers().len(),
            "catalog": catalog_items,
            "snippets": invoicer_core::snippets::snippets(index.site()).len(),
            // What the templates page lists: the books' own, and the
            // starters not copied in (4d).
            "templates": super::templates::count_in(&session),
            "nextNumber": next.number.to_string(),
            // What it follows, and where that is (H19): `document`, `books`
            // (booked in the journal, no file) or `start`.
            "nextNumberAfter": next.after,
            "nextNumberSource": next.source.as_str(),
        }))
    })
    .await
}

/// `GET /api/invoices`' query: the invoice list's URL contract (PLAN 5b,
/// S1, with R14's `paid_from`/`paid_to`), plus `type`, `open`, `overdue`
/// and `today`. A blank value is no
/// filter, so the SPA can pass its URL through as it is.
#[derive(Debug, Default, Deserialize)]
struct ListParams {
    /// `draft`, `sent`, `paid`, `void`, `open` or `overdue` (or any other
    /// `status:` value, `estimate-open`).
    status: Option<String>,
    /// A customer's name, title or `[[link]]`.
    customer: Option<String>,
    #[serde(rename = "type")]
    doc_type: Option<String>,
    #[serde(default)]
    open: bool,
    #[serde(default)]
    overdue: bool,
    /// Dated on or after (YYYY-MM-DD).
    from: Option<String>,
    /// Dated on or before.
    to: Option<String>,
    /// Money received on or after (YYYY-MM-DD; PLAN R14).
    paid_from: Option<String>,
    /// Money received on or before.
    paid_to: Option<String>,
    /// Open and this many days past due: `current`, `31-60`, `91+`.
    aged: Option<String>,
    /// Number, customer, reference or total holding this text.
    q: Option<String>,
    /// `number`, `customer`, `date`, `due`, `total` or `status`.
    sort: Option<String>,
    /// `asc` (the default) or `desc`.
    dir: Option<String>,
    today: Option<String>,
}

/// A query value, `None` when absent or blank.
pub(crate) fn given(value: Option<&String>) -> Option<&str> {
    value.map(|v| v.trim()).filter(|v| !v.is_empty())
}

/// `?dir=`: `asc` (the default) or `desc`.
pub(crate) fn sort_dir(value: Option<&String>) -> Result<views::SortDir, ApiError> {
    given(value).map_or(Ok(views::SortDir::Asc), |d| {
        views::SortDir::parse(d).ok_or_else(|| bad("dir", d))
    })
}

impl ListParams {
    /// The query these parameters ask for, or why they don't read.
    fn query(&self) -> Result<ListQuery, ApiError> {
        let date = |name: &str, value: Option<&String>| -> Result<Option<String>, ApiError> {
            given(value)
                .map(|d| {
                    if is_iso_date(d) {
                        Ok(d.to_string())
                    } else {
                        Err(ApiError::bad_request(format!(
                            "{name}={d}: not a date (YYYY-MM-DD)"
                        )))
                    }
                })
                .transpose()
        };
        let mut query = ListQuery {
            filter: ListFilter {
                doc_type: given(self.doc_type.as_ref())
                    .map(|t| DocType::parse(t).ok_or_else(|| bad("type", t)))
                    .transpose()?,
                status: None,
                customer: given(self.customer.as_ref()).map(str::to_string),
            },
            open: self.open,
            overdue: self.overdue,
            from: date("from", self.from.as_ref())?,
            to: date("to", self.to.as_ref())?,
            paid_from: date("paid_from", self.paid_from.as_ref())?,
            paid_to: date("paid_to", self.paid_to.as_ref())?,
            aged: given(self.aged.as_ref())
                .map(|a| views::Aged::parse(a).ok_or_else(|| bad("aged", a)))
                .transpose()?,
            text: given(self.q.as_ref()).map(str::to_string),
            sort: given(self.sort.as_ref())
                .map(|s| views::InvoiceSort::parse(s).ok_or_else(|| bad("sort", s)))
                .transpose()?,
            dir: sort_dir(self.dir.as_ref())?,
            ..ListQuery::default()
        };
        if let Some(status) = given(self.status.as_ref()) {
            query = match views::ListStatus::parse(status) {
                Some(wanted) => query.with_status(wanted),
                None => {
                    query.filter.status =
                        Some(Status::parse(status).ok_or_else(|| bad("status", status))?);
                    query
                }
            };
        }
        Ok(query)
    }
}

/// `GET /api/invoices`: `list --json`, filtered and sorted by the query
/// ([`ListParams`]).
async fn invoices(State(state): State<AppState>, Query(params): Query<ListParams>) -> ApiResult {
    let today = today(params.today.as_deref())?;
    let mut query = params.query()?;
    with_books(&state, StatusCode::OK, move |books| {
        let session = books.session();
        // The dashboard links a customer by the books' name (R5).
        if let Some(name) = &query.filter.customer {
            query.filter.customer = Some(views::customer_filter_name(session.index(), name));
        }
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

pub(crate) fn bad(what: &str, value: &str) -> ApiError {
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
    with_books(&state, StatusCode::OK, move |books| {
        let session = books.session();
        let doc = session
            .find(&number)
            .map_err(|e| ApiError::status(StatusCode::NOT_FOUND, e.to_string()))?;
        let mut problems = views::document_problems(&session);
        if let Some(number) = doc.number() {
            problems.extend(views::books_problems(&session, Some(&[number.to_string()])));
        }
        let detail = views::show(&session, doc, &problems, &today)
            .map_err(|e| ApiError::status(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        to_value(detail)
    })
    .await
}

/// `GET /api/invoices/{number}/books`: the Books panel (what the journal
/// holds under the invoice's number), by number or path.
async fn invoice_books(State(state): State<AppState>, Path(number): Path<String>) -> ApiResult {
    with_books(&state, StatusCode::OK, move |books| {
        let session = books.session();
        let doc = session
            .find(&number)
            .map_err(|e| ApiError::status(StatusCode::NOT_FOUND, e.to_string()))?;
        to_value(views::invoice_books(&session, doc))
    })
    .await
}

/// `POST /api/preview`: render an invoice (unsaved text, or the saved file)
/// through its template or another ([`preview`]). Writes nothing.
async fn preview(
    State(state): State<AppState>,
    payload: Result<Json<PreviewRequest>, JsonRejection>,
) -> ApiResult {
    let Json(request) = payload.map_err(|e| ApiError::bad_request(e.body_text()))?;
    let books = state.open_books()?;
    let rendered = tokio::task::spawn_blocking(move || preview::render(&books, request))
        .await
        .map_err(|e| {
            ApiError::status(
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

    let books = state.open_books()?;
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

/// `GET /api/ui-prefs`: the effective theme and scale. Reads only.
async fn ui_prefs(State(state): State<AppState>) -> ApiResult {
    blocking(StatusCode::OK, move || to_value(prefs::current(&state))).await
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
    blocking(StatusCode::OK, move || {
        to_value(prefs::apply(&state, &patch)?)
    })
    .await
}

/// `POST /api/ui-prefs/pin`: `{key, pinned}`, keep one of the shared keys
/// (the theme, the scale, the payment matcher's three) for the open books
/// in their `ledgeline.toml [ui]`, creating the file if needed, or stop
/// keeping it; Ledgeline's route, answering the effective values after.
async fn pin_ui_pref(
    State(state): State<AppState>,
    payload: Result<Json<Value>, JsonRejection>,
) -> ApiResult {
    let Json(body) = payload.map_err(|e| ApiError::bad_request(e.body_text()))?;
    let pin = Pin::from_json(&prefs::SCHEMA, &body)?;
    blocking(StatusCode::OK, move || to_value(prefs::pin(&state, pin)?)).await
}

/// `POST /api/session`: the cookie MBR's pages need.
async fn session(Extension(gate): Extension<Arc<MbrGate>>) -> Response {
    gate.session_response()
}
