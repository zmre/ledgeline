// The SPA's view state (plan 38 WP-5): types, defaults, bounds and the pure
// helpers the settings store uses to load, migrate and save it.
//
// These used to be persisted in localStorage under `ledgeline.settings.v1`. The
// desktop app's WebView origin carries a port that changes on every launch, so
// every relaunch forgot them; they now live server-side, in `ledgeline.toml`
// beside the journal over `ui-prefs.toml` in the config directory
// (`GET`/`PATCH /api/ui-prefs`, crates/ledgeline-server/src/ui_prefs.rs). Only
// `serverUrl`/`serverToken` stay in localStorage: they describe this client,
// not the ledger.
//
// Plain TypeScript (no runes), so nativeDecode.ts can name these types and
// node-only tests can exercise every helper.

import {PIE_DIMENSIONS, type PieDimension} from "#lib/holdings/profileTypes.js";

/** Journal table column toggles (defaults per WP-03: Date, Status, Description, Accounts, Amount). */
export interface ColumnConfig {
    date: boolean;
    status: boolean;
    description: boolean;
    accounts: boolean;
    amount: boolean;
}

export const COLUMN_KEYS: readonly (keyof ColumnConfig)[] = ["date", "status", "description", "accounts", "amount"];

/** The insights box's two views. Persisted, so the tab you left it on is the tab you come back to. */
export type InsightsTab = "activity" | "balances";

export const INSIGHTS_TABS: readonly InsightsTab[] = ["activity", "balances"];

/**
 * Every UI preference. The engine owns the same list, defaults and bounds
 * (`SPECS` in ui_prefs.rs); `the defaults are the SPA's` there pins them.
 */
export interface UiPrefs {
    columns: ColumnConfig;
    insightsOpen: boolean;
    /** Which view the insights box shows. "activity" (the period P&L) is the default; "balances" is the cash snapshot. */
    insightsTab: InsightsTab;
    /** Balances view: leave out accounts that currently hold nothing. On by default — a closed account is noise, not news. */
    hideZeroBalances: boolean;
    /** The P&L tab's two Sankey panels, one flag each. They collapse independently. */
    flowsInOpen: boolean;
    flowsOutOpen: boolean;
    /** The chart panels above the Net Worth and Cash Flow tables, one flag each, open by default. */
    netWorthChartOpen: boolean;
    cashFlowChartOpen: boolean;
    /**
     * The Budget tab's "not budgeted" section. Closed by default, and the flag
     * is what GATES ITS FETCH — a section nobody has opened costs no request.
     * One flag for both lists inside it (income and expenses).
     */
    budgetGapsOpen: boolean;
    /** Insights' "More metrics" list under the startup boxes (plans/37). Closed by default. */
    startupMoreOpen: boolean;
    /** What the Holdings pie is divided by: one slice per holding (the default), or per asset class / sector / … */
    holdingsPieDimension: PieDimension;
    /**
     * The Projections tab's last-loaded scenario FILE, by its relative id
     * (`plans/projection-series-a.journal`), reopened on the next mount. Null
     * when nothing has been loaded, which is the seed-from-the-journal case. A
     * stale id that no longer names a file is not an error state — the tab
     * falls back to seeding and clears it.
     */
    lastProjectionId: string | null;
    /** Which benchmark lines the Stocks value-over-time chart overlays, by catalog symbol. None by default. */
    benchmarks: string[];
    /** Review Recents' cap (plan 32 D19): how many unreviewed transactions one run takes. Default 50. */
    reviewRecentLimit: number;
    /** Both review queues' lookback window in days, and the journal's "unreviewed" stripe window (D4, D20). `0` means no limit. */
    reviewMaxDays: number;
    /** The journal's review stripes (D20). On by default. */
    reviewMarkers: boolean;
    /** Find Matches (plan 35 D8): how many days apart two halves of a transfer may be. Default 10. */
    matchWindow: number;
    /** Find Matches: only clearing postings this many days back are proposed. `0` means no limit. Default 365. */
    matchLookback: number;
    /** The "In transit" finding's threshold (D10). Default 10. */
    matchTransitDays: number;
}

export type UiPrefKey = keyof UiPrefs;

export const UI_PREF_KEYS: readonly UiPrefKey[] = [
    "columns",
    "insightsOpen",
    "insightsTab",
    "hideZeroBalances",
    "flowsInOpen",
    "flowsOutOpen",
    "netWorthChartOpen",
    "cashFlowChartOpen",
    "budgetGapsOpen",
    "startupMoreOpen",
    "holdingsPieDimension",
    "lastProjectionId",
    "benchmarks",
    "reviewRecentLimit",
    "reviewMaxDays",
    "reviewMarkers",
    "matchWindow",
    "matchLookback",
    "matchTransitDays",
];

/** A partial update, as `PATCH /api/ui-prefs` takes it: `columns` may name only some members. */
export type UiPrefsPatch = Partial<Omit<UiPrefs, "columns">> & {columns?: Partial<ColumnConfig>};

/**
 * Where the engine keeps this session's UI preferences.
 *  - `ledger`: `ledgeline.toml` beside the journal (the normal case).
 *  - `memory`: no journal file behind the session; saves last until the engine stops.
 *  - `browser`: the engine leaves them to this client's localStorage (the e2e
 *    and contract harnesses, and any engine without the route).
 */
export type UiPrefsStorage = "ledger" | "memory" | "browser";

export const UI_PREFS_STORAGES: readonly UiPrefsStorage[] = ["ledger", "memory", "browser"];

/** Something in a settings file the engine did not use as written. */
export interface UiPrefsIssue {
    readonly file: string;
    /** The key as spelled in that file (`ui.matchWindow`), or null for the whole file. */
    readonly key: string | null;
    readonly message: string;
}

/** The `GET`/`PATCH /api/ui-prefs` answer. */
export interface UiPrefsState {
    readonly prefs: UiPrefs;
    readonly storage: UiPrefsStorage;
    /** The ledger already has UI preferences; the one-time localStorage migration runs only when it does not. */
    readonly ledgerHasUi: boolean;
    readonly issues: readonly UiPrefsIssue[];
}

/** Review Recents' default cap, and the queues' default window (D19). */
export const REVIEW_RECENT_LIMIT = 50;
export const REVIEW_MAX_DAYS = 365;
/** Bounds the two review numbers are clamped to: a run of 1–1000, a window of 0 (no limit)–36500 days. */
export const REVIEW_LIMIT_RANGE = [1, 1000] as const;
export const REVIEW_DAYS_RANGE = [0, 36500] as const;

/** Find Matches' defaults and bounds (plan 35 D8): a window of 1–60 days, a lookback of 0 (no limit)–36500, in transit after 1–365. */
export const MATCH_WINDOW = 10;
export const MATCH_LOOKBACK = 365;
export const MATCH_TRANSIT_DAYS = 10;
export const MATCH_WINDOW_RANGE = [1, 60] as const;
export const MATCH_LOOKBACK_RANGE = [0, 36500] as const;
export const MATCH_TRANSIT_RANGE = [1, 365] as const;

/**
 * Ticked benchmark symbols as kept: ticker-shaped strings, deduplicated, in the
 * order they were ticked. Membership in the catalog is NOT checked here — the
 * catalog is the engine's and is not known at startup — so the chart restricts
 * these to the catalog before it asks (`catalogOrder`).
 */
const TICKER = /^[A-Z0-9.^=-]{1,12}$/;

/** A whole number inside `range`, or `fallback` for anything else (a stale blob, a hand edit). */
export function wholeIn(value: unknown, range: readonly [number, number], fallback: number): number {
    if (typeof value !== "number" || !Number.isFinite(value)) return fallback;
    return Math.min(range[1], Math.max(range[0], Math.round(value)));
}

export function cleanBenchmarks(value: unknown): string[] {
    const picked = Array.isArray(value) ? value.filter((v): v is string => typeof v === "string" && TICKER.test(v)) : [];
    return [...new Set(picked)];
}

export const defaultColumns = (): ColumnConfig => ({date: true, status: true, description: true, accounts: true, amount: true});

export const defaultUiPrefs = (): UiPrefs => ({
    columns: defaultColumns(),
    insightsOpen: true,
    insightsTab: "activity",
    hideZeroBalances: true,
    flowsInOpen: true,
    flowsOutOpen: true,
    netWorthChartOpen: true,
    cashFlowChartOpen: true,
    budgetGapsOpen: false,
    startupMoreOpen: false,
    holdingsPieDimension: "holding",
    lastProjectionId: null,
    benchmarks: [],
    reviewRecentLimit: REVIEW_RECENT_LIMIT,
    reviewMaxDays: REVIEW_MAX_DAYS,
    reviewMarkers: true,
    matchWindow: MATCH_WINDOW,
    matchLookback: MATCH_LOOKBACK,
    matchTransitDays: MATCH_TRANSIT_DAYS,
});

const flag = (value: unknown, fallback: boolean): boolean => (typeof value === "boolean" ? value : fallback);

/**
 * Whatever `parsed` holds, read as UI preferences: each key checked and, where
 * it is absent or unusable, its default. What the localStorage blob has always
 * been read through, so a blob from any older version loads.
 */
export function cleanUiPrefs(parsed: Record<string, unknown>): UiPrefs {
    const base = defaultUiPrefs();
    const columns = typeof parsed.columns === "object" && parsed.columns !== null ? (parsed.columns as Record<string, unknown>) : {};
    return {
        columns: Object.fromEntries(COLUMN_KEYS.map((key) => [key, flag(columns[key], base.columns[key])])) as unknown as ColumnConfig,
        insightsOpen: flag(parsed.insightsOpen, base.insightsOpen),
        // Validated against the union rather than merely typechecked as a
        // string: a stale blob naming a tab that no longer exists would
        // otherwise render an empty box with no way back.
        insightsTab: INSIGHTS_TABS.includes(parsed.insightsTab as InsightsTab) ? (parsed.insightsTab as InsightsTab) : base.insightsTab,
        hideZeroBalances: flag(parsed.hideZeroBalances, base.hideZeroBalances),
        flowsInOpen: flag(parsed.flowsInOpen, base.flowsInOpen),
        flowsOutOpen: flag(parsed.flowsOutOpen, base.flowsOutOpen),
        netWorthChartOpen: flag(parsed.netWorthChartOpen, base.netWorthChartOpen),
        cashFlowChartOpen: flag(parsed.cashFlowChartOpen, base.cashFlowChartOpen),
        budgetGapsOpen: flag(parsed.budgetGapsOpen, base.budgetGapsOpen),
        startupMoreOpen: flag(parsed.startupMoreOpen, base.startupMoreOpen),
        // Validated against the union, like `insightsTab`: a dimension a later
        // version dropped must fall back rather than select nothing.
        holdingsPieDimension: PIE_DIMENSIONS.includes(parsed.holdingsPieDimension as PieDimension)
            ? (parsed.holdingsPieDimension as PieDimension)
            : base.holdingsPieDimension,
        // A file id has no closed set to validate against, and the engine
        // re-validates its shape on every request anyway.
        lastProjectionId: typeof parsed.lastProjectionId === "string" && parsed.lastProjectionId !== "" ? parsed.lastProjectionId : null,
        benchmarks: cleanBenchmarks(parsed.benchmarks),
        reviewRecentLimit: wholeIn(parsed.reviewRecentLimit, REVIEW_LIMIT_RANGE, REVIEW_RECENT_LIMIT),
        reviewMaxDays: wholeIn(parsed.reviewMaxDays, REVIEW_DAYS_RANGE, REVIEW_MAX_DAYS),
        reviewMarkers: flag(parsed.reviewMarkers, base.reviewMarkers),
        matchWindow: wholeIn(parsed.matchWindow, MATCH_WINDOW_RANGE, MATCH_WINDOW),
        matchLookback: wholeIn(parsed.matchLookback, MATCH_LOOKBACK_RANGE, MATCH_LOOKBACK),
        matchTransitDays: wholeIn(parsed.matchTransitDays, MATCH_TRANSIT_RANGE, MATCH_TRANSIT_DAYS),
    };
}

/**
 * The UI preferences an old localStorage blob actually holds, cleaned, as a
 * patch — only the keys present (and, for `columns`, only the members
 * present), so the one-time migration writes what the user had and nothing
 * the defaults would give them anyway. Empty when the blob holds none.
 */
export function legacyUiPatch(parsed: Record<string, unknown>): UiPrefsPatch {
    const cleaned = cleanUiPrefs(parsed);
    const patch: Record<string, unknown> = {};
    for (const key of UI_PREF_KEYS) {
        if (!(key in parsed)) continue;
        if (key === "columns") {
            const stored = typeof parsed.columns === "object" && parsed.columns !== null ? (parsed.columns as Record<string, unknown>) : {};
            const members = Object.fromEntries(
                COLUMN_KEYS.filter((member) => typeof stored[member] === "boolean").map((member) => [member, cleaned.columns[member]])
            );
            if (Object.keys(members).length > 0) patch.columns = members;
        } else {
            patch[key] = cleaned[key];
        }
    }
    return patch as UiPrefsPatch;
}

/** `parsed` with every UI preference removed — what stays in localStorage once they live on the engine. */
export function withoutUiFields(parsed: Record<string, unknown>): Record<string, unknown> {
    return Object.fromEntries(Object.entries(parsed).filter(([key]) => !(UI_PREF_KEYS as readonly string[]).includes(key)));
}

/** Merge `patch` into `into` (a pending save), `columns` member by member. */
export function mergePatch(into: UiPrefsPatch, patch: UiPrefsPatch): UiPrefsPatch {
    const merged: UiPrefsPatch = {...into, ...patch};
    if (into.columns !== undefined || patch.columns !== undefined) merged.columns = {...into.columns, ...patch.columns};
    return merged;
}
