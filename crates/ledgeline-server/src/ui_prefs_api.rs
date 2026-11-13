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
use crate::ui_prefs::{self, Patch, Pin};

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
    let wake = patch.touches_auto_prices();
    let saver = state.clone();
    let Json(body) = compute(move || ui_prefs::apply(&saver, &patch)).await?;
    if wake {
        // Turning automatic prices on (or widening its weeks) should not wait
        // up to an hour to show it works.
        state.auto_prices().wake();
    }
    Ok(no_store_json(body))
}

/// `POST /api/ui-prefs/pin` — keep one setting for the open ledger only, or
/// stop: body `{"key": "theme" | "uiScale", "pinned": bool}`. Pinning writes
/// the key's current effective value into the ledger's `ledgeline.toml`
/// `[ui]`, creating the file if there is none — the one time the app creates
/// it, because the user asked. Unpinning removes the key and keeps the file.
/// Answers like `GET`, so `localKeys` shows the pin.
///
/// `400` for any other key or shape; `409` in browser mode, with no ledger
/// file open, or when an existing `ledgeline.toml` cannot be safely written
/// (symlinked, too large, not TOML).
pub(crate) async fn pin(
    State(state): State<AppState>,
    payload: Result<Json<serde_json::Value>, JsonRejection>,
) -> Result<Response, AppError> {
    let pin = Pin::from_json(&json_body(payload)?)?;
    let saver = state.clone();
    let Json(body) = compute(move || ui_prefs::pin(&saver, pin)).await?;
    // A pin can change this ledger's effective auto-price setting; a spare
    // wake costs one local check.
    state.auto_prices().wake();
    Ok(no_store_json(body))
}
