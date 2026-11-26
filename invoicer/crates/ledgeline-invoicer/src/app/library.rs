//! The catalog and snippets editors (invoicer PLAN 4c). The writes are the
//! core's ([`invoicer_core::catalog_edit`], [`invoicer_core::snippet_edit`]),
//! so the CLI can share them.
//!
//! | Route                            | Does                                                                |
//! |----------------------------------|---------------------------------------------------------------------|
//! | `GET /api/catalog`               | Every catalog note: its table as a grid, its `hash`, its problems; `path` (`[invoicer] catalog`). |
//! | `POST /api/catalog`              | Create the empty catalog at `[invoicer] catalog` → `201` [`CatalogSaved`]. |
//! | `PUT /api/catalog/{*path}`       | `{hash, table}`: write the note's table → [`CatalogSaved`].         |
//! | `GET /api/snippets`              | Every snippet: title, kind, body, `hash`, and `used_by`.            |
//! | `POST /api/snippets`             | `{title, kind?, body?}` → `201` [`SnippetSaved`].                    |
//! | `PUT /api/snippets/{*path}`      | `{hash, fields: {title?, kind?, body?}}` → [`SnippetSaved`].        |
//! | `DELETE /api/snippets/{*path}`   | `{hash, force?}` → `{deleted}`.                                     |
//!
//! `path` is the note's path in the books folder, the `.md` optional.
//! Errors are `{"error": "…", "code": "…"}`: `changed` (409, the note isn't
//! the bytes `hash` names; `revision` is its hash now), `invalid` (422; a
//! catalog's cells come in `problems`: `[{row, column, message}]`),
//! `name-taken` / `exists` (409), `in-use` (409, `customers`: delete with
//! `force`), `not-found` (404), `bad-request` (400), `storage` (500).
//!
//! [`CatalogSaved`]: invoicer_core::catalog_edit::CatalogSaved
//! [`SnippetSaved`]: invoicer_core::snippet_edit::SnippetSaved

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use invoicer_core::catalog_edit::{self, CatalogError};
use invoicer_core::editor::GridForm;
use invoicer_core::snippet_edit::{self, SnippetError, SnippetFields};
use ledgeline_core::vault::VaultPath;
use serde::Deserialize;
use serde_json::{Value, json};

use super::AppState;
use super::api::{ApiError, ApiResult, body, to_value, with_books};
use super::books::OpenBooks;

/// The catalog and snippet routes (state still to be supplied).
pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/catalog", get(catalogs).post(create_catalog))
        .route("/api/catalog/{*path}", axum::routing::put(save_catalog))
        .route("/api/snippets", get(snippets).post(create_snippet))
        .route(
            "/api/snippets/{*path}",
            axum::routing::put(update_snippet).delete(delete_snippet),
        )
}

impl From<CatalogError> for ApiError {
    fn from(error: CatalogError) -> Self {
        let message = error.to_string();
        match error {
            CatalogError::Invalid(problems) => {
                Self::new(StatusCode::UNPROCESSABLE_ENTITY, "invalid", message)
                    .with("problems", json!(problems))
            }
            CatalogError::Table(_) | CatalogError::NotText(_) => {
                Self::new(StatusCode::UNPROCESSABLE_ENTITY, "invalid", message)
            }
            CatalogError::NotFound(_) | CatalogError::NotACatalog(_) => {
                Self::new(StatusCode::NOT_FOUND, "not-found", message)
            }
            CatalogError::Changed { current, .. } => {
                Self::new(StatusCode::CONFLICT, "changed", message)
                    .with("revision", Value::String(current))
            }
            CatalogError::Exists(_) => Self::new(StatusCode::CONFLICT, "exists", message),
            CatalogError::Vault(_) => {
                Self::new(StatusCode::INTERNAL_SERVER_ERROR, "storage", message)
            }
        }
    }
}

impl From<SnippetError> for ApiError {
    fn from(error: SnippetError) -> Self {
        let message = error.to_string();
        match error {
            SnippetError::Invalid(_) | SnippetError::Frontmatter(_) | SnippetError::NotText(_) => {
                Self::new(StatusCode::UNPROCESSABLE_ENTITY, "invalid", message)
            }
            SnippetError::NotFound(_) | SnippetError::NotASnippet(_) => {
                Self::new(StatusCode::NOT_FOUND, "not-found", message)
            }
            SnippetError::NameTaken { .. } => {
                Self::new(StatusCode::CONFLICT, "name-taken", message)
            }
            SnippetError::Exists(_) => Self::new(StatusCode::CONFLICT, "exists", message),
            SnippetError::Changed { current, .. } => {
                Self::new(StatusCode::CONFLICT, "changed", message)
                    .with("revision", Value::String(current))
            }
            SnippetError::InUse { customers } => Self::new(StatusCode::CONFLICT, "in-use", message)
                .with("customers", json!(customers)),
            SnippetError::Vault(_) => {
                Self::new(StatusCode::INTERNAL_SERVER_ERROR, "storage", message)
            }
        }
    }
}

/// The note `key` names: its path in the books folder, `.md` optional.
fn note_path(books: &OpenBooks, key: &str) -> Result<VaultPath, ApiError> {
    let trimmed = key.trim().trim_start_matches('/');
    let session = books.session();
    let site = session.index().site();
    [trimmed.to_string(), format!("{trimmed}.md")]
        .into_iter()
        .filter_map(|p| VaultPath::new(&p).ok())
        .find(|p| site.note(p).is_some())
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                "not-found",
                format!("{trimmed}: no such note"),
            )
        })
}

/// `GET /api/catalog`.
async fn catalogs(State(state): State<AppState>) -> ApiResult {
    with_books(&state, StatusCode::OK, |books| {
        let session = books.session();
        Ok(json!({
            "catalogs": to_value(catalog_edit::list(session.index(), session.vault()))?,
            "path": session.settings().catalog,
        }))
    })
    .await
}

/// `POST /api/catalog`: the empty catalog at `[invoicer] catalog`.
async fn create_catalog(State(state): State<AppState>) -> ApiResult {
    with_books(&state, StatusCode::CREATED, |books| {
        let mut session = books.session();
        let (index, vault) = session.parts_mut();
        to_value(catalog_edit::create(index, vault)?)
    })
    .await
}

/// `PUT /api/catalog/{path}`'s body.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogBody {
    /// The content hash the grid was read from.
    hash: String,
    /// The table.
    table: GridForm,
}

/// `PUT /api/catalog/{path}`: write the note's table.
async fn save_catalog(
    State(state): State<AppState>,
    Path(key): Path<String>,
    payload: Result<Json<CatalogBody>, JsonRejection>,
) -> ApiResult {
    let request = body(payload)?;
    with_books(&state, StatusCode::OK, move |books| {
        let path = note_path(books, &key)?;
        let mut session = books.session();
        let (index, vault) = session.parts_mut();
        to_value(catalog_edit::save(
            index,
            vault,
            &path,
            &request.hash,
            &request.table,
        )?)
    })
    .await
}

/// `GET /api/snippets`.
async fn snippets(State(state): State<AppState>) -> ApiResult {
    with_books(&state, StatusCode::OK, |books| {
        let session = books.session();
        to_value(snippet_edit::list(session.index(), session.vault()))
    })
    .await
}

/// `POST /api/snippets`.
async fn create_snippet(
    State(state): State<AppState>,
    payload: Result<Json<SnippetFields>, JsonRejection>,
) -> ApiResult {
    let fields = body(payload)?;
    with_books(&state, StatusCode::CREATED, move |books| {
        let mut session = books.session();
        let (index, vault) = session.parts_mut();
        to_value(snippet_edit::create(index, vault, &fields)?)
    })
    .await
}

/// `PUT /api/snippets/{path}`'s body.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SnippetBody {
    /// The content hash the form was filled from.
    hash: String,
    /// What to change; absent fields are left.
    fields: SnippetFields,
}

/// `PUT /api/snippets/{path}`.
async fn update_snippet(
    State(state): State<AppState>,
    Path(key): Path<String>,
    payload: Result<Json<SnippetBody>, JsonRejection>,
) -> ApiResult {
    let request = body(payload)?;
    with_books(&state, StatusCode::OK, move |books| {
        let path = note_path(books, &key)?;
        let mut session = books.session();
        let (index, vault) = session.parts_mut();
        to_value(snippet_edit::update(
            index,
            vault,
            &path,
            &request.hash,
            &request.fields,
        )?)
    })
    .await
}

/// `DELETE /api/snippets/{path}`'s body.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteBody {
    /// The content hash the page shows.
    hash: String,
    /// Delete even though customers name it.
    #[serde(default)]
    force: bool,
}

/// `DELETE /api/snippets/{path}`.
async fn delete_snippet(
    State(state): State<AppState>,
    Path(key): Path<String>,
    payload: Result<Json<DeleteBody>, JsonRejection>,
) -> ApiResult {
    let request = body(payload)?;
    with_books(&state, StatusCode::OK, move |books| {
        let path = note_path(books, &key)?;
        let mut session = books.session();
        let (index, vault) = session.parts_mut();
        snippet_edit::delete(index, vault, &path, &request.hash, request.force)?;
        Ok(json!({ "deleted": path.to_string() }))
    })
    .await
}
