//! The receivables dashboard (invoicer PLAN Phase 5a).
//!
//! | Route                       | Does                                                                      |
//! |-----------------------------|---------------------------------------------------------------------------|
//! | `GET /api/dashboard?today=&paid_days=` | [`invoicer_core::views::dashboard::Dashboard`], plus `open_ledgeline`: whether this process can open the books in Ledgeline. `paid_days` is the paid window (1–3660; `[invoicer] paid-window-days` when absent). |
//! | `POST /api/ledgeline/open`  | Open the books' main journal in Ledgeline (the window, macOS); `503` elsewhere, `500` when Ledgeline isn't installed. |
//!
//! Ledgeline has no URL scheme to open a page by, so "Open in Ledgeline"
//! opens the books themselves; recognition is posted from its accrual view.

use axum::Router;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use invoicer_core::settings::MAX_PAID_WINDOW_DAYS;
use invoicer_core::views::dashboard;
use serde::Deserialize;
use serde_json::{Value, json};

use super::AppState;
use super::api::{ApiError, ApiResult, blocking, to_value, today, with_books};
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
    /// The "paid in the last N days" window; `paid-window-days` by default.
    paid_days: Option<String>,
}

/// `GET /api/dashboard`.
async fn show(State(state): State<AppState>, Query(params): Query<Params>) -> ApiResult {
    let today = today(params.today.as_deref())?;
    let paid_days = super::api::given(params.paid_days.as_ref())
        .map(|d| {
            d.parse::<u32>()
                .ok()
                .filter(|n| (1..=MAX_PAID_WINDOW_DAYS).contains(n))
                .ok_or_else(|| {
                    ApiError::bad_request(format!(
                        "paid_days={d}: give a number of days from 1 to {MAX_PAID_WINDOW_DAYS}"
                    ))
                })
        })
        .transpose()?;
    // A file's modification time is dated in local time, as `today` is.
    let utc_offset = i64::from(chrono::Local::now().offset().local_minus_utc());
    let can_open = state
        .native()
        .is_some_and(|n| n.capabilities().open_ledgeline);
    with_books(&state, StatusCode::OK, move |books| {
        let session = books.session();
        let mut options = dashboard::DashboardOptions::of(session.settings());
        options.utc_offset = utc_offset;
        if let Some(days) = paid_days {
            options.paid_days = days;
        }
        let mut body: Value = to_value(dashboard::dashboard_for(&session, &today, &options))?;
        body["open_ledgeline"] = json!(can_open);
        Ok(body)
    })
    .await
}

/// `POST /api/ledgeline/open`: the open books in Ledgeline.
async fn open_ledgeline(State(state): State<AppState>) -> ApiResult {
    let native = native_for(&state, |c| c.open_ledgeline, "Opening Ledgeline")?;
    let books = state.open_books()?;
    blocking(StatusCode::OK, move || {
        native
            .open_in_ledgeline(books.journal())
            .map_err(|e| ApiError::status(StatusCode::INTERNAL_SERVER_ERROR, e))?;
        Ok(json!({ "done": true }))
    })
    .await
}
