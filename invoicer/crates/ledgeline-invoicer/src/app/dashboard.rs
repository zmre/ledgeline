//! The receivables dashboard (invoicer PLAN Phase 5a).
//!
//! | Route                       | Does                                                                      |
//! |-----------------------------|---------------------------------------------------------------------------|
//! | `GET /api/dashboard?today=` | [`invoicer_core::views::dashboard::Dashboard`], plus `open_ledgeline`: whether this process can open the books in Ledgeline. |
//! | `POST /api/ledgeline/open`  | Open the books' main journal in Ledgeline (the window, macOS); `503` elsewhere, `500` when Ledgeline isn't installed. |
//!
//! Ledgeline has no URL scheme to open a page by, so "Open in Ledgeline"
//! opens the books themselves; recognition is posted from its accrual view.

use axum::Router;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use invoicer_core::views::dashboard;
use serde::Deserialize;
use serde_json::{Value, json};

use super::AppState;
use super::api::{ApiError, ApiResult, blocking, no_books, to_value, today, with_books};
use super::pdf::native_for;

/// The dashboard routes (state still to be supplied).
pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/dashboard", get(show))
        .route("/api/ledgeline/open", post(open_ledgeline))
}

/// `GET /api/dashboard`'s query.
#[derive(Debug, Default, Deserialize)]
struct Params {
    /// For "overdue", aging and what is due (YYYY-MM-DD); the local date
    /// by default.
    today: Option<String>,
}

/// `GET /api/dashboard`.
async fn show(State(state): State<AppState>, Query(params): Query<Params>) -> ApiResult {
    let today = today(params.today.as_deref())?;
    let can_open = state
        .native()
        .is_some_and(|n| n.capabilities().open_ledgeline);
    with_books(&state, move |books| {
        let session = books.session();
        let mut body: Value = to_value(dashboard::dashboard_for(&session, &today))?;
        body["open_ledgeline"] = json!(can_open);
        Ok(body)
    })
    .await
}

/// `POST /api/ledgeline/open`: the open books in Ledgeline.
async fn open_ledgeline(State(state): State<AppState>) -> ApiResult {
    let native = native_for(&state, |c| c.open_ledgeline, "Opening Ledgeline")?;
    let books = state.books().ok_or_else(no_books)?;
    blocking(move || {
        native
            .open_in_ledgeline(books.journal())
            .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e))?;
        Ok(json!({ "done": true }))
    })
    .await
}
