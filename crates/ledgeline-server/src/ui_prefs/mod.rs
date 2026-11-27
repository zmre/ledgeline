//! Ledgeline's UI preferences, bound to the server's state.
//!
//! The data model — the files, the merge, where a save goes, the trust model
//! and the theme pin — is [`ledgeline_core::ui_prefs`]'s, shared with every
//! app built on the shell. This module is only Ledgeline's binding of it:
//! [`ledgeline::SCHEMA`](prefs::ledgeline::SCHEMA) as the schema, the
//! [`AppState`]'s session, config folder and open ledger as the
//! [`Context`](prefs::Context), the core error mapped onto [`AppError`], and
//! the typed readers Ledgeline's background workers poll.

use ledgeline_core::ui_prefs::{self as prefs, LedgerSlot, Schema, UiPrefsError};
use serde_json::Value as Json;

pub(crate) use prefs::ledgeline::{
    AUTO_PRICE_KEYS, AUTO_RECOGNIZE_KEYS, DEFAULT_SYNC_SOURCE_DIR, SYNC_KEYS,
};
pub use prefs::{BootAppearance, Patch, Pin, UiPrefsSession, WireUiPrefs};

use crate::AppState;
use crate::error::AppError;

/// Ledgeline's preferences.
pub(crate) const SCHEMA: &Schema = &prefs::ledgeline::SCHEMA;

/// Env switch for browser storage: the session keeps nothing in files. Any
/// non-empty value turns it on.
pub const UI_PREFS_IN_BROWSER_ENV: &str = "LEDGELINE_UI_PREFS_IN_BROWSER";

impl From<UiPrefsError> for AppError {
    fn from(error: UiPrefsError) -> Self {
        match error {
            UiPrefsError::BadRequest(message) => Self::BadRequest(message),
            UiPrefsError::Conflict(message) => Self::Conflict(message),
            UiPrefsError::Internal(message) => Self::Internal(message),
        }
    }
}

/// One operation's view of `state`: Ledgeline's schema, the session, the
/// config folder's `ui-prefs.toml` and the open journal's `ledgeline.toml`.
fn context(state: &AppState) -> prefs::Context<'_> {
    let location = state.snapshot().storage();
    let ledger = location
        .as_deref()
        .map_or(LedgerSlot::NoLedger, |location| {
            LedgerSlot::beside(location.vault().clone(), &location.root_dir())
        });
    prefs::Context {
        schema: SCHEMA,
        session: state.ui_prefs(),
        global_file: state.app_dirs().config_file(prefs::GLOBAL_FILE),
        ledger,
        // The main journal's key in its vault: its absolute path on the
        // desktop. Only ever a TOML table name, never sent to the client.
        ledger_id: location
            .as_deref()
            .map(|location| location.key(location.main()).to_string_lossy().into_owned()),
    }
}

/// The effective preferences for `state`'s current journal. Reads only.
pub(crate) fn current(state: &AppState) -> WireUiPrefs {
    prefs::current(&context(state))
}

/// Save `patch` and answer with the effective values ([`prefs::apply`]).
pub(crate) fn apply(state: &AppState, patch: &Patch) -> Result<WireUiPrefs, AppError> {
    Ok(prefs::apply(&context(state), patch)?)
}

/// Pin or unpin one key for the open ledger ([`prefs::pin`]).
pub(crate) fn pin(state: &AppState, pin: Pin) -> Result<WireUiPrefs, AppError> {
    Ok(prefs::pin(&context(state), pin)?)
}

/// The open ledger's `ledgeline.toml` as text ([`prefs::ledger_text`]).
pub(crate) fn ledger_text(state: &AppState) -> Result<Option<String>, AppError> {
    Ok(prefs::ledger_text(&context(state))?)
}

/// Rewrite the open ledger's `ledgeline.toml` with `edit`, creating it under
/// bank sync's header when missing ([`prefs::edit_ledger_text`]).
pub(crate) fn edit_ledger_text(
    state: &AppState,
    edit: impl Fn(&str) -> Result<String, AppError>,
) -> Result<(), AppError> {
    prefs::edit_ledger_text(&context(state), prefs::ledgeline::LEDGER_SYNC_HEADER, edit)
}

/// The effective theme and scale for `state`'s current journal, for the
/// desktop app's first paint. Reads only, like [`current`].
pub fn boot_appearance(state: &AppState) -> BootAppearance {
    prefs::boot_appearance(&context(state))
}

/// The automatic price updater's two settings, effective for `state`'s
/// current journal (the ledger's `ledgeline.toml` over `ui-prefs.toml` over
/// the defaults, like every other key). Reads only, like [`current`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AutoPricePrefs {
    pub(crate) enabled: bool,
    /// Already clamped to `0..=52` by the spec.
    pub(crate) weeks: u32,
}

/// See [`AutoPricePrefs`]. In browser mode — the e2e and live-contract
/// harnesses — this is the default, off, so those never reach the network.
pub(crate) fn auto_price_prefs(state: &AppState) -> AutoPricePrefs {
    let wire = current(state);
    AutoPricePrefs {
        enabled: wire
            .prefs()
            .get("autoPrices")
            .and_then(Json::as_bool)
            .unwrap_or(false),
        weeks: wire
            .prefs()
            .get("autoPricesWeeks")
            .and_then(Json::as_u64)
            .and_then(|weeks| u32::try_from(weeks).ok())
            .unwrap_or(0),
    }
}

/// The `returnIncomeAccounts` setting (plan 48 C6), effective for `state`'s
/// current journal: which income accounts' credits are return on the
/// holdings they buy. Reads only, like [`current`].
pub(crate) fn return_income(state: &AppState) -> ledgeline_core::holdings::ReturnIncome {
    current(state)
        .prefs()
        .get("returnIncomeAccounts")
        .and_then(Json::as_array)
        .map(|names| {
            ledgeline_core::holdings::ReturnIncome::new(names.iter().filter_map(Json::as_str))
        })
        .unwrap_or_default()
}

/// Bank sync's four settings, effective for `state`'s current journal. In
/// browser mode — the e2e and live-contract harnesses — these are the
/// defaults, so auto-sync is off and nothing reaches the network unasked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SyncSettings {
    pub(crate) auto_sync: bool,
    /// Clamped to `3..=24` by the spec.
    pub(crate) every_hours: u32,
    /// Checked by [`ledgeline_core::ui_prefs::check_rel_dir`]; still resolved
    /// inside the journal's folder before use.
    pub(crate) source_dir: String,
    /// `0` = keep forever.
    pub(crate) keep_days: u32,
    /// `manualRemindDays`: a manual account's row turns amber after this
    /// many days with no new transaction.
    pub(crate) remind_days: u32,
}

/// Whether `autoRecognize` (plan 45) is on for `state`'s journal. Off in
/// browser mode, like every background writer.
pub(crate) fn auto_recognize(state: &AppState) -> bool {
    current(state)
        .prefs()
        .get("autoRecognize")
        .and_then(Json::as_bool)
        .unwrap_or(false)
}

/// See [`SyncSettings`].
pub(crate) fn sync_settings(state: &AppState) -> SyncSettings {
    let wire = current(state);
    let whole = |key: &str, default: u32| {
        wire.prefs()
            .get(key)
            .and_then(Json::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(default)
    };
    SyncSettings {
        auto_sync: wire
            .prefs()
            .get("autoSync")
            .and_then(Json::as_bool)
            .unwrap_or(false),
        every_hours: whole("autoSyncEveryHours", 6),
        source_dir: wire
            .prefs()
            .get("syncSourceDir")
            .and_then(Json::as_str)
            .unwrap_or(DEFAULT_SYNC_SOURCE_DIR)
            .to_string(),
        keep_days: whole("syncKeepSourceDays", 0),
        remind_days: whole("manualRemindDays", 35),
    }
}
