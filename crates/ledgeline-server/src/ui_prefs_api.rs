//! `GET`/`PATCH /api/ui-prefs` — the SPA's view state, kept in the global
//! `ui-prefs.toml`, overridden key by key by a hand-made `ledgeline.toml`
//! beside the journal, over the built-in defaults. The files, the merge, where
//! a save goes and the trust model are [`crate::ui_prefs`]'s.
//!
//! Both answer the same body ([`ui_prefs::WireUiPrefs`]): every preference's
//! effective value, where they are kept, which keys the ledger's own file
//! sets, and what in the files was not used as written.

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
/// Reads only: never creates or writes a file.
pub(crate) async fn get(State(state): State<AppState>) -> Result<Response, AppError> {
    let Json(body) = compute(move || Ok(ui_prefs::current(&state))).await?;
    Ok(no_store_json(body))
}

/// `PATCH /api/ui-prefs` — set the named keys, and only those: each in the
/// ledger's `ledgeline.toml` when it already sets that key, else in
/// `ui-prefs.toml`; answers with the effective values after.
///
/// `400` for an unknown key or a bad value (nothing written); `409` when
/// `ui-prefs.toml` exists but cannot be safely rewritten (too large, not TOML,
/// a group that is not a table). A symlinked `ui-prefs.toml` is not an error:
/// the save is kept for the session and reported in `issues`.
pub(crate) async fn patch(
    State(state): State<AppState>,
    payload: Result<Json<serde_json::Value>, JsonRejection>,
) -> Result<Response, AppError> {
    let patch = Patch::from_json(&json_body(payload)?)?;
    let Json(body) = compute(move || ui_prefs::apply(&state, &patch)).await?;
    Ok(no_store_json(body))
}
