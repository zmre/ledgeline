//! Customers (invoicer PLAN 4b): the list, one customer's page, creating
//! one and editing one. The writes are the core's
//! ([`invoicer_core::customer_edit`]), so the CLI can share them.
//!
//! | Route                          | Does                                                        |
//! |--------------------------------|-------------------------------------------------------------|
//! | `GET /api/customers`           | `customers list --json`, each row with its `account` (`today`). |
//! | `POST /api/customers`          | Create one: `CustomerFields` → `201` [`CustomerSaved`].     |
//! | `GET /api/customers/{*key}`    | `customers show --json` plus `account`, `hash`, `form`, `backlinks` (`today`). |
//! | `PUT /api/customers/{*key}`    | `{hash, fields}`: edit by key → [`CustomerSaved`].          |
//!
//! `key` is the note's path in the books folder (`customers/Fabrikam.md`,
//! the `.md` optional) or a name `customers show` takes (title, alias, file
//! name, `[[wikilink]]`).
//!
//! Errors: `400` a body that doesn't read (an unknown field, a password in
//! the portal), `404` no such customer, `409` the note changed on disk since
//! `hash` (or a name already taken, or the file exists), `422` a field that
//! doesn't read or a frontmatter layout the editor can't change safely
//! (edit it by hand).

use std::path::Path as FsPath;
use std::time::Duration;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Router, response::Response};
use invoicer_core::customer_edit::{self, CustomerError, CustomerFields};
use invoicer_core::customers::{Customer, CustomerLookup, is_customer};
use invoicer_core::notes::Note;
use invoicer_core::session::Session;
use invoicer_core::views::{self, account};
use ledgeline_core::vault::VaultPath;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::AppState;
use super::api::{ApiError, ApiResult, blocking, no_books, to_value, today, with_books};
use super::books::OpenBooks;

/// The customer routes (state still to be supplied).
pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/customers", get(list).post(create))
        .route("/api/customers/{*key}", get(detail).put(update))
}

#[derive(Debug, Default, Deserialize)]
struct TodayParam {
    today: Option<String>,
}

/// A refusal from the core, as an API error.
fn customer_error(error: &CustomerError) -> ApiError {
    let status = match error {
        CustomerError::Invalid(_) | CustomerError::Frontmatter(_) | CustomerError::NotText(_) => {
            StatusCode::UNPROCESSABLE_ENTITY
        }
        CustomerError::NotFound(_) | CustomerError::NotACustomer { .. } => StatusCode::NOT_FOUND,
        CustomerError::NameTaken { .. }
        | CustomerError::Exists(_)
        | CustomerError::Changed { .. } => StatusCode::CONFLICT,
        CustomerError::Vault(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    ApiError(status, error.to_string())
}

/// `GET /api/customers`: every customer, by title, with its account.
async fn list(State(state): State<AppState>, Query(params): Query<TodayParam>) -> ApiResult {
    let today = today(params.today.as_deref())?;
    with_books(&state, move |books| {
        to_value(account::customer_accounts(&books.session(), &today))
    })
    .await
}

/// `POST /api/customers`: create a customer note in `customers-dir`.
/// `201` with [`CustomerSaved`]: its `link` (`[[Title]]`) is what an
/// invoice's `customer:` writes.
async fn create(
    State(state): State<AppState>,
    body: Result<Json<CustomerFields>, JsonRejection>,
) -> Result<Response, ApiError> {
    let Json(fields) = body.map_err(|e| ApiError::bad_request(e.body_text()))?;
    let books = state.books().ok_or_else(no_books)?;
    let mut response = blocking(move || {
        let mut session = books.session();
        let (index, vault) = session.parts_mut();
        let saved = customer_edit::create(index, vault, &fields).map_err(|e| customer_error(&e))?;
        to_value(saved)
    })
    .await?
    .into_response();
    *response.status_mut() = StatusCode::CREATED;
    Ok(response)
}

/// The customer note `key` names: a path in the books folder (`.md`
/// optional), else a name.
fn find(session: &Session, key: &str) -> Result<VaultPath, ApiError> {
    let index = session.index();
    let words = &index.settings().words;
    let trimmed = key.trim().trim_start_matches('/');
    let by_path = [trimmed.to_string(), format!("{trimmed}.md")]
        .into_iter()
        .filter_map(|p| VaultPath::new(&p).ok())
        .find_map(|p| index.site().note(&p));
    if let Some(note) = by_path {
        if !is_customer(note, words) {
            return Err(ApiError(
                StatusCode::NOT_FOUND,
                format!(
                    "{} isn't a customer: a person or organization note tagged `{}`",
                    note.path, words.customer_tag
                ),
            ));
        }
        return Ok(note.path.clone());
    }
    views::find_customer(index, trimmed)
        .map(|c| c.contact.path)
        .map_err(|e| match e {
            CustomerLookup::NotFound => ApiError(
                StatusCode::NOT_FOUND,
                format!("no customer is called {key:?}"),
            ),
            CustomerLookup::NotACustomer(path) => ApiError(
                StatusCode::NOT_FOUND,
                format!(
                    "{path} isn't a person or organization tagged `{}`",
                    words.customer_tag
                ),
            ),
        })
}

/// One note linking to the customer (MBR-CHANGES M1: body links and
/// frontmatter links both).
#[derive(Debug, Serialize)]
struct Backlink {
    /// MBR's page for it (`/invoices/2026-11-10 Customer X Invoice 105/`).
    url: String,
    /// The note in the books folder, when MBR's index has it.
    path: Option<String>,
    /// Its title.
    title: Option<String>,
    /// The frontmatter key that links (`customer`); `None` for a body link.
    key: Option<String>,
}

/// How long a customer's page waits for MBR's first scan of the folder
/// before answering without backlinks.
const SCAN_WAIT: Duration = Duration::from_secs(5);

/// The notes MBR's index says link to `path`. Empty until MBR's first scan
/// of the folder is complete: a backlink index built over a partial scan
/// would miss notes (the caller waits for it, [`SCAN_WAIT`]).
fn backlinks(books: &OpenBooks, path: &VaultPath) -> Vec<Backlink> {
    let site = books.mbr_site();
    if !site.repo().is_scan_complete() {
        return Vec::new();
    }
    let inbound = site.backlinks(FsPath::new(path.as_str()));
    if inbound.is_empty() {
        return Vec::new();
    }
    let notes: Vec<_> = site.notes().collect();
    inbound
        .into_iter()
        .map(|link| {
            let note = notes.iter().find(|n| n.url == link.from);
            Backlink {
                path: note.map(|n| n.path.to_string_lossy().replace('\\', "/")),
                title: note.map(|n| n.title.clone()),
                key: link.origin.map(|o| match o {
                    mbr_core::link_index::LinkOrigin::Frontmatter { key } => key,
                }),
                url: link.from,
            }
        })
        .collect()
}

/// `GET /api/customers/{key}`: `customers show --json` (its invoices, newest
/// first, with status, total, due and open/overdue), plus:
///
/// - `account`: balance, overdue count, last payment;
/// - `hash`: the note's content hash, which a `PUT` names;
/// - `form`: the editable fields as the note writes them
///   ([`CustomerFields`]);
/// - `backlinks`: every note MBR's index says links here (empty if MBR's
///   first scan of the folder hasn't finished within [`SCAN_WAIT`]).
async fn detail(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Query(params): Query<TodayParam>,
) -> ApiResult {
    let today = today(params.today.as_deref())?;
    if let Some(books) = state.books() {
        // MBR scans the folder in the background once the books open.
        let site = books.mbr_site();
        let _ = tokio::time::timeout(SCAN_WAIT, site.repo().wait_for_scan()).await;
    }
    with_books(&state, move |books| {
        let path = {
            let mut session = books.session();
            let path = find(&session, &key)?;
            // Read it afresh, so `hash` and `form` are the file as it is
            // now, not as the watcher last saw it.
            let (index, vault) = session.parts_mut();
            index.refresh(vault, std::slice::from_ref(&path));
            path
        };
        let links = backlinks(books, &path);
        let session = books.session();
        let note: Note = session
            .index()
            .site()
            .note(&path)
            .cloned()
            .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, format!("{path}: no such note")))?;
        let customer = Customer::from_note(&note)
            .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, format!("{path}: not a contact")))?;
        let problems = views::document_problems(&session);
        let detail = views::customer_detail(&session, &customer, &problems, &today);
        let open = account::open_items(&session, &today);
        let mut value = to_value(&detail)?;
        if let Value::Object(map) = &mut value {
            map.insert(
                "account".into(),
                to_value(account::customer_account(&session, &customer, &open))?,
            );
            map.insert("hash".into(), json!(note.stamp.fingerprint.token()));
            map.insert(
                "form".into(),
                to_value(customer_edit::fields_of(&customer, &note))?,
            );
            map.insert("backlinks".into(), to_value(&links)?);
        }
        Ok(value)
    })
    .await
}

/// `PUT /api/customers/{key}`'s body.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateBody {
    /// The content hash the form was filled from (`GET`'s `hash`).
    hash: String,
    /// The fields to write; absent ones are left as they are.
    fields: CustomerFields,
}

/// `PUT /api/customers/{key}`: edit the note by key ([`CustomerSaved`]).
/// `409` when the note is no longer the bytes `hash` names.
async fn update(
    State(state): State<AppState>,
    Path(key): Path<String>,
    body: Result<Json<UpdateBody>, JsonRejection>,
) -> ApiResult {
    let Json(body) = body.map_err(|e| ApiError::bad_request(e.body_text()))?;
    let books = state.books().ok_or_else(no_books)?;
    blocking(move || {
        let mut session = books.session();
        let path = find(&session, &key)?;
        let (index, vault) = session.parts_mut();
        let saved = customer_edit::update(index, vault, &path, &body.hash, &body.fields)
            .map_err(|e| customer_error(&e))?;
        to_value(saved)
    })
    .await
}
