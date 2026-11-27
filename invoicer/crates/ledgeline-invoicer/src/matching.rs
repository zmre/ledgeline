//! The invoicer's UI preferences schema, and the payment matcher's
//! effective settings from it (RC-10, settings plan D2), for the CLI and
//! the app alike.
//!
//! The matcher's three keys are Ledgeline's (`settlementLookbackDays`,
//! `settlementToleranceBps`, `settlementLeadDays`), read the way Ledgeline
//! reads them: the books' `ledgeline.toml [ui]` when it pins one, else the
//! shared `ui-prefs.toml`, else the default. So `apply-payment` lists the
//! same deposits the app does, and both agree with Ledgeline.
//!
//! The invoicer shows and sets only the lookback and the lead. The tolerance
//! stays in the schema, and pinnable, because the file is shared and
//! Ledgeline writes it; the invoicer's matcher (`apply::candidates`) offers a
//! short deposit as a part payment whatever it is.

use invoicer_core::session::Session;
use ledgeline_core::accrual::settle::SettleSettings;
use ledgeline_core::ui_prefs::{
    self as core, LedgerSlot, SETTLEMENT_LEAD_DAYS, SETTLEMENT_LOOKBACK_DAYS,
    SETTLEMENT_TOLERANCE_BPS, Schema, THEME, TIME_LONG_RUNNING, TIME_OTHER, TIME_PERSON,
    TIME_PINNED, TIME_STEP, TIME_WEEKLY_TARGET, UI_SCALE, UiPrefsSession,
};
use ledgeline_core::vault::{Vault, VaultPath};
use ledgeline_shell::AppDirs;
use std::sync::Arc;

/// The invoicer's preferences: the theme and scale, and the matcher's
/// three keys, all shared with Ledgeline and all pinnable; and the time
/// tracker's `[time]` (plan 46 Settings), the invoicer's own, whose pins and
/// slot 0 are kept per ledger (account names belong to one ledger).
pub(crate) static SCHEMA: Schema = Schema {
    specs: &[
        THEME,
        UI_SCALE,
        SETTLEMENT_LOOKBACK_DAYS,
        SETTLEMENT_TOLERANCE_BPS,
        SETTLEMENT_LEAD_DAYS,
        TIME_PERSON,
        TIME_PINNED,
        TIME_OTHER,
        TIME_STEP,
        TIME_LONG_RUNNING,
        TIME_WEEKLY_TARGET,
    ],
    pinnable: &[
        "theme",
        "uiScale",
        "settlementLookbackDays",
        "settlementToleranceBps",
        "settlementLeadDays",
    ],
    per_ledger: &["time.pinned", "time.other"],
    // The time routes read these themselves, so they must hold in browser
    // storage too (e2e, a browser on `--server`).
    server_side: &[
        "time.person",
        "time.pinned",
        "time.other",
        "time.step",
        "time.long-running",
        "time.weekly-target",
    ],
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
