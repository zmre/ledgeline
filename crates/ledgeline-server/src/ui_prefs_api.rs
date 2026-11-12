//! `GET`/`PATCH /api/ui-prefs` — the SPA's view state, kept in `ledgeline.toml`
//! beside the journal over `ui-prefs.toml` and the built-in defaults. The
//! files, the merge and the trust model are [`crate::ui_prefs`]'s.
//!
//! Both answer the same body ([`ui_prefs::WireUiPrefs`]): every preference's
//! effective value, where they are kept, whether the ledger already has any,
//! and what in the files was not used as written.

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::response::Response;

use crate::AppState;
use crate::edit_api::json_body;
use crate::error::AppError;
use crate::reports_api::compute;
use crate::security::no_store_json;
use crate::ui_prefs::{self, Patch};

/// `GET /api/ui-prefs` — the effective preferences for the open journal.
pub(crate) async fn get(State(state): State<AppState>) -> Result<Response, AppError> {
    let Json(body) = compute(move || Ok(ui_prefs::current(&state))).await?;
    Ok(no_store_json(body))
}

/// `PATCH /api/ui-prefs` — set the named keys, and only those, where this
/// session keeps its preferences; answers with the effective values after.
///
/// `400` for an unknown key or a bad value (nothing written); `409` when the
/// ledger's file exists but cannot be safely rewritten (a symlink, too large,
/// not TOML, a `ui` that is not a table).
pub(crate) async fn patch(
    State(state): State<AppState>,
    payload: Result<Json<serde_json::Value>, JsonRejection>,
) -> Result<Response, AppError> {
    let patch = Patch::from_json(&json_body(payload)?)?;
    let Json(body) = compute(move || ui_prefs::apply(&state, &patch)).await?;
    Ok(no_store_json(body))
}
