//! Ledgeline's own UI preferences: its [`SCHEMA`] for the engine in
//! [`super`], and the key lists its background workers wake on.
//!
//! The bounds and defaults are the SPA's (`web/src/lib/settings/`), which is
//! why this table, not the engine, is what changes when the SPA gains a view
//! setting.

use super::{
    Kind, SETTLEMENT_LEAD_DAYS, SETTLEMENT_LOOKBACK_DAYS, SETTLEMENT_TOLERANCE_BPS, Schema, Spec,
    THEME, UI_SCALE, flag, whole,
};

/// The insights box's two views (`InsightsTab` in `settings.svelte.ts`).
const INSIGHTS_TABS: &[&str] = &["activity", "balances"];

/// What the Holdings pie can be divided by (`PIE_DIMENSIONS` in
/// `web/src/lib/holdings/profileTypes.ts`).
const PIE_DIMENSIONS: &[&str] = &[
    "holding",
    "assetClass",
    "sector",
    "industry",
    "securityType",
    "category",
    "risk",
];

/// The keys [`super::pin`] may write into [`super::LEDGER_FILE`]: what a
/// person would want to tell one ledger from another by. The automatic price
/// update keys (plan 41) are here because "fetch prices for my real books, but
/// not for the sample ledger I demo with" is exactly a per-ledger choice.
///
/// The bank sync keys (plan 43) are here for the same reason: sync the real
/// books automatically, not the demo ledger.
///
/// The invoice matcher's three keys (plan 45 D-settle) are here because
/// how far back to look and how early a payment may come differ by books
/// (settings plan D2): Ledgeline Invoicer pins them too, and reads them
/// for its Apply payment.
pub const PINNABLE: &[&str] = &[
    "theme",
    "uiScale",
    "autoPrices",
    "autoPricesWeeks",
    "autoSync",
    "autoSyncEveryHours",
    "syncSourceDir",
    "syncKeepSourceDays",
    "manualRemindDays",
    "autoRecognize",
    "settlementLookbackDays",
    "settlementToleranceBps",
    "settlementLeadDays",
];

/// The automatic price keys, whose change wakes the price updater.
pub const AUTO_PRICE_KEYS: &[&str] = &["autoPrices", "autoPricesWeeks"];

/// The sync keys, whose change wakes the sync scheduler.
pub const SYNC_KEYS: &[&str] = &[
    "autoSync",
    "autoSyncEveryHours",
    "syncSourceDir",
    "syncKeepSourceDays",
];

/// The background recognition switch (plan 45), whose change wakes the
/// recognizer.
pub const AUTO_RECOGNIZE_KEYS: &[&str] = &["autoRecognize"];

/// `syncSourceDir`'s default: where fetched CSVs are kept, relative to the
/// journal's folder.
pub const DEFAULT_SYNC_SOURCE_DIR: &str = "imports/sync";

/// Every preference, with the defaults `settings.svelte.ts` has always used.
/// The bounds are the SPA's (`REVIEW_LIMIT_RANGE` and friends).
const SPECS: &[Spec] = &[
    flag("columns.date", true),
    flag("columns.status", true),
    flag("columns.description", true),
    flag("columns.accounts", true),
    flag("columns.amount", true),
    flag("insightsOpen", true),
    Spec {
        key: "insightsTab",
        kind: Kind::Choice {
            allowed: INSIGHTS_TABS,
            default: "activity",
        },
    },
    flag("hideZeroBalances", true),
    flag("flowsInOpen", true),
    flag("flowsOutOpen", true),
    flag("netWorthChartOpen", true),
    flag("cashFlowChartOpen", true),
    flag("budgetGapsOpen", false),
    flag("startupMoreOpen", false),
    Spec {
        key: "holdingsPieDimension",
        kind: Kind::Choice {
            allowed: PIE_DIMENSIONS,
            default: "holding",
        },
    },
    Spec {
        key: "lastProjectionId",
        kind: Kind::FileId,
    },
    Spec {
        key: "benchmarks",
        kind: Kind::Tickers,
    },
    whole("reviewRecentLimit", 1, 1000, 50),
    whole("reviewMaxDays", 0, 36500, 365),
    flag("reviewMarkers", true),
    whole("matchWindow", 1, 60, 10),
    whole("matchLookback", 0, 36500, 365),
    whole("matchTransitDays", 1, 365, 10),
    // The two common keys, in the place the wire has always listed them.
    THEME,
    UI_SCALE,
    // Plan 41: the engine's background price updater (`auto_prices`). Off
    // until asked for — it makes network requests on the user's behalf.
    flag("autoPrices", false),
    // How many past Monday–Sunday weeks it backfills a missing weekly close
    // for. 0 = the latest close only. A year is plenty; every extra week only
    // makes the one request per symbol a little larger.
    whole("autoPricesWeeks", 0, 52, 6),
    // Plan 43: bank sync in the background. Off until asked for — it fetches
    // from the bank feed and writes to the journal on the user's behalf.
    flag("autoSync", false),
    // The floor of 3 protects SimpleFIN's ~24-requests-a-day quota.
    whole("autoSyncEveryHours", 3, 24, 6),
    Spec {
        key: "syncSourceDir",
        kind: Kind::RelDir {
            default: DEFAULT_SYNC_SOURCE_DIR,
        },
    },
    // 0 = keep every fetched CSV forever, the default: nothing is deleted
    // unless the user asks for it.
    whole("syncKeepSourceDays", 0, 3650, 0),
    // Days without a new transaction before a manually imported account's
    // row turns amber on Imports → Accounts.
    whole("manualRemindDays", 1, 3650, 35),
    // Plan 45: post due recognition entries in the background. Off until
    // asked for — it writes to the journal on the user's behalf.
    flag("autoRecognize", false),
    // Plan 45 D8: how the journal shows posted recognition entries.
    Spec {
        key: "recognitionRows",
        kind: Kind::Choice {
            allowed: &["group", "hide", "show"],
            default: "group",
        },
    },
    // Plan 45 D-settle: the invoice ↔ payment matcher, beside the match
    // settings; shared with Ledgeline Invoicer (see the specs).
    SETTLEMENT_LOOKBACK_DAYS,
    SETTLEMENT_TOLERANCE_BPS,
    SETTLEMENT_LEAD_DAYS,
];

/// The keys [`super::GLOBAL_FILE`] keeps per ledger
/// ([`Schema::per_ledger`]). `lastProjectionId` is a file id relative to the
/// journal's folder: kept at top level, the last scenario opened on ANY
/// ledger — another book, a fixture a development build had open — was
/// reopened on this one, where it does not exist.
pub const PER_LEDGER: &[&str] = &["lastProjectionId"];

/// Ledgeline's preferences.
pub const SCHEMA: Schema = Schema {
    specs: SPECS,
    pinnable: PINNABLE,
    per_ledger: PER_LEDGER,
    server_side: &[],
};

/// The top of a [`super::LEDGER_FILE`] that linking a bank account creates
/// ([`super::edit_ledger_text`]).
pub const LEDGER_SYNC_HEADER: &str = concat!(
    "# Ledgeline settings for this ledger. [sync] lists the bank accounts\n",
    "# Ledgeline syncs into it; credentials are never stored here. [ui] keys\n",
    "# override ui-prefs.toml while this ledger is open. Docs: \"Settings files\".\n",
    "\n",
);
