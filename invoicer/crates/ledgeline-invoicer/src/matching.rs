//! The invoicer's UI preferences schema, and the payment matcher's
//! effective settings from it (RC-10, settings plan D2), for the CLI and
//! the app alike.
//!
//! The matcher's three keys are Ledgeline's (`settlementLookbackDays`,
//! `settlementToleranceBps`, `settlementLeadDays`), read the way Ledgeline
//! reads them: the books' `ledgeline.toml [ui]` when it pins one, else the
//! shared `ui-prefs.toml`, else the default. So `apply-payment` lists the
//! same deposits the app does, and both agree with Ledgeline.

use invoicer_core::session::Session;
use ledgeline_core::accrual::settle::SettleSettings;
use ledgeline_core::ui_prefs::{
    self as core, LedgerSlot, SETTLEMENT_LEAD_DAYS, SETTLEMENT_LOOKBACK_DAYS,
    SETTLEMENT_TOLERANCE_BPS, Schema, THEME, UI_SCALE, UiPrefsSession,
};
use ledgeline_core::vault::{Vault, VaultPath};
use ledgeline_shell::AppDirs;
use std::sync::Arc;

/// The invoicer's preferences: the theme and scale, and the matcher's
/// three keys, all shared with Ledgeline and all pinnable.
pub(crate) static SCHEMA: Schema = Schema {
    specs: &[
        THEME,
        UI_SCALE,
        SETTLEMENT_LOOKBACK_DAYS,
        SETTLEMENT_TOLERANCE_BPS,
        SETTLEMENT_LEAD_DAYS,
    ],
    pinnable: &[
        "theme",
        "uiScale",
        "settlementLookbackDays",
        "settlementToleranceBps",
        "settlementLeadDays",
    ],
    per_ledger: &[],
};

/// Where the books' `ledgeline.toml` is, and how `ui-prefs.toml` keys the
/// books' own values: beside the main journal `main` in `vault`.
pub(crate) fn ledger_of(vault: &Arc<dyn Vault>, main: &VaultPath) -> (LedgerSlot, String) {
    (
        LedgerSlot::beside(Arc::clone(vault), &main.parent().unwrap_or_default()),
        vault.key(main).to_string_lossy().into_owned(),
    )
}

/// The matcher's effective settings for the books `session` has open, read
/// from the files (never browser mode: the CLI has no browser).
pub(crate) fn settle_settings(app_dirs: &AppDirs, session: &Session) -> SettleSettings {
    let (ledger, ledger_id) = ledger_of(&session.shared_vault(), session.main());
    let prefs = UiPrefsSession::default();
    let ctx = core::Context {
        schema: &SCHEMA,
        session: &prefs,
        global_file: app_dirs.config_file(core::GLOBAL_FILE),
        ledger,
        ledger_id: Some(ledger_id),
    };
    core::settle_settings(core::current(&ctx).prefs())
}
