//! The invoicer's UI preferences, bound to its state: the same files,
//! merge and save routing as Ledgeline's ([`ledgeline_core::ui_prefs`]),
//! so the theme and scale are shared (ARCHITECTURE §2.3). The global
//! `ui-prefs.toml` is the shared config folder's, and the open books'
//! `ledgeline.toml [ui]` pin applies to both apps.
//!
//! The two common keys and the payment matcher's three (shared with
//! Ledgeline, settings plan D2), all pinnable: [`crate::matching::SCHEMA`].
//! A key only Ledgeline's schema names is neither read nor written here, and
//! survives this app's saves.

use ledgeline_core::accrual::settle::SettleSettings;
use ledgeline_core::ui_prefs::{
    self as core, LedgerSlot, Patch, Pin, UiPrefsError, UiPrefsSession, WireUiPrefs,
};

use super::AppState;

pub(crate) use crate::matching::SCHEMA;

/// The e2e switch Ledgeline's harnesses use: any non-empty value leaves UI
/// preferences to the browser, so a run over a committed fixture writes no
/// settings file.
pub(crate) const IN_BROWSER_ENV: &str = "LEDGELINE_UI_PREFS_IN_BROWSER";

/// The session [`IN_BROWSER_ENV`] asks for.
pub(crate) fn session_from_env() -> UiPrefsSession {
    if std::env::var_os(IN_BROWSER_ENV).is_some_and(|v| !v.is_empty()) {
        UiPrefsSession::browser()
    } else {
        UiPrefsSession::default()
    }
}

/// One operation's view of `state`, run inside `f`.
fn with_context<T>(state: &AppState, f: impl FnOnce(&core::Context<'_>) -> T) -> T {
    let books = state.books();
    let (ledger, ledger_id) = match books.as_deref() {
        Some(books) => {
            let (slot, id) = crate::matching::ledger_of(books.vault(), books.main());
            (slot, Some(id))
        }
        None => (LedgerSlot::NoLedger, None),
    };
    let ctx = core::Context {
        schema: &SCHEMA,
        session: state.ui_prefs(),
        global_file: state.app_dirs().config_file(core::GLOBAL_FILE),
        ledger,
        ledger_id,
    };
    f(&ctx)
}

/// The effective preferences. Reads only.
pub(crate) fn current(state: &AppState) -> WireUiPrefs {
    with_context(state, core::current)
}

/// The payment matcher's effective settings (RC-10): the books' pin, else
/// the shared `ui-prefs.toml`, else the defaults; the defaults in browser
/// mode (e2e), as every other preference.
pub(crate) fn settle_settings(state: &AppState) -> SettleSettings {
    core::settle_settings(current(state).prefs())
}

/// Keep `pin`'s key for the open books, or stop: `ui_prefs::pin`'s answer.
pub(crate) fn pin(state: &AppState, pin: Pin) -> Result<WireUiPrefs, UiPrefsError> {
    with_context(state, |ctx| core::pin(ctx, pin))
}

/// Rewrite the open books' `ledgeline.toml` with `edit` under the prefs
/// lock ([`core::edit_ledger_text`]), creating it with `header` when
/// missing: the Settings page's Save.
pub(crate) fn edit_ledger_text<E: From<UiPrefsError>>(
    state: &AppState,
    header: &str,
    edit: impl Fn(&str) -> Result<String, E>,
) -> Result<(), E> {
    with_context(state, |ctx| core::edit_ledger_text(ctx, header, edit))
}

/// Save `patch`; answers the effective values after.
pub(crate) fn apply(state: &AppState, patch: &Patch) -> Result<WireUiPrefs, UiPrefsError> {
    with_context(state, |ctx| core::apply(ctx, patch))
}

/// The theme and scale for the window's first paint.
#[cfg(any(feature = "gui", test))]
pub(crate) fn boot_appearance(state: &AppState) -> core::BootAppearance {
    with_context(state, core::boot_appearance)
}
