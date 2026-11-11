// Settings store (WP-02): Svelte 5 runes state persisted to localStorage
// under a versioned key. `setServerUrl` verifies GET /version before persisting.

import {HledgerApi, SETTINGS_STORAGE_KEY} from "#lib/api/client.js";
import {PIE_DIMENSIONS, type PieDimension} from "#lib/holdings/profileTypes.js";

/** Journal table column toggles (defaults per WP-03: Date, Status, Description, Accounts, Amount). */
export interface ColumnConfig {
    date: boolean;
    status: boolean;
    description: boolean;
    accounts: boolean;
    amount: boolean;
}

const defaultColumns = (): ColumnConfig => ({date: true, status: true, description: true, accounts: true, amount: true});

/** The insights box's two views. Persisted, so the tab you left it on is the tab you come back to. */
export type InsightsTab = "activity" | "balances";

const INSIGHTS_TABS: readonly InsightsTab[] = ["activity", "balances"];

interface PersistedSettings {
    serverUrl: string | null;
    /**
     * Access token for a CROSS-ORIGIN engine (vite dev, the e2e harness), which
     * the engine prints at startup and takes from $LEDGELINE_TOKEN. Embedded
     * (same-origin) pages ignore this: the GUI hands its WebView the token out
     * of band and a headless engine prints a `#token=` launch URL, and
     * `apiToken()` in api/client.ts prefers either of those. Null when no
     * token was entered.
     */
    serverToken: string | null;
    columns: ColumnConfig;
    insightsOpen: boolean;
    /** Which view the insights box shows. "activity" (the period P&L) is the default; "balances" is the cash snapshot. */
    insightsTab: InsightsTab;
    /** Balances view: leave out accounts that currently hold nothing. On by default — a closed account is noise, not news. */
    hideZeroBalances: boolean;
    /**
     * The P&L tab's two Sankey panels, one flag each. They collapse
     * independently: one flag made both arrows move together.
     */
    flowsInOpen: boolean;
    flowsOutOpen: boolean;
    /**
     * The chart panels above the Net Worth and Cash Flow tables, one flag each,
     * open by default.
     */
    netWorthChartOpen: boolean;
    cashFlowChartOpen: boolean;
    /**
     * The Budget tab's "not budgeted" section. Closed by default, and the flag
     * is what GATES ITS FETCH — a section nobody has opened costs no request.
     *
     * One flag for both lists inside it (income and expenses): they answer one
     * question, and collapsing half of "how much does my budget miss" is not a
     * state worth remembering.
     */
    budgetGapsOpen: boolean;
    /** Insights' "More metrics" list under the startup boxes (plans/37). Closed by default. */
    startupMoreOpen: boolean;
    /**
     * What the Holdings pie is divided by: one slice per holding (the
     * default), or one per asset class / sector / … (see
     * `#lib/holdings/profileTypes.js`). Per-browser view state, like
     * `insightsTab`.
     */
    holdingsPieDimension: PieDimension;
    /**
     * The Projections tab's last-loaded scenario FILE, by its relative id
     * (`plans/projection-series-a.journal`), reopened on the next mount.
     *
     * HERE rather than in `prefs.json` (plan 22, decision 10): `WirePrefs` is
     * `deny_unknown_fields` and `PUT /api/prefs` replaces the whole object with
     * no PATCH, so any older client that saved preferences would silently clear
     * the field. This is per-browser view state, which is what this store is
     * for.
     *
     * Null when nothing has been loaded, which is the seed-from-the-journal
     * case. A stale id that no longer names a file is not an error state — the
     * tab falls back to seeding and clears it.
     */
    lastProjectionId: string | null;
    /**
     * Which benchmark lines the Stocks value-over-time chart overlays, by
     * catalog symbol. Per browser, like every other view toggle here; none by
     * default, because each one is a request that may reach Yahoo Finance.
     */
    benchmarks: string[];
    /**
     * Review Recents' cap (plan 32 D19): how many unreviewed transactions one
     * run takes, newest first. Default 50.
     */
    reviewRecentLimit: number;
    /**
     * Both review queues' lookback window in days, and the window the journal's
     * faint "unreviewed" row stripe marks (D4, D20). `0` means no limit.
     * Default 365.
     */
    reviewMaxDays: number;
    /** The journal's review stripes (D20). On by default. */
    reviewMarkers: boolean;
    /** Find Matches (plan 35 D8): how many days apart two halves of a transfer may be. Default 10. */
    matchWindow: number;
    /** Find Matches: only clearing postings this many days back are proposed. `0` means no limit. Default 365. */
    matchLookback: number;
    /**
     * The "In transit" finding's threshold (D10): a clearing posting with no
     * partner this many days old is flagged. Default 10. Stored for the
     * Problems check; the engine applies its own default until the check
     * takes a parameter.
     */
    matchTransitDays: number;
}

/** Keep only catalog symbols, once each, in catalog order. */
/**
 * Ticked benchmark symbols as persisted: ticker-shaped strings, deduplicated, in
 * the order they were ticked. Membership in the catalog is NOT checked here — the
 * catalog is the engine's (`/api/holdings/benchmarks/catalog`) and is not known
 * at startup — so the chart restricts these to the catalog, in slot order, before
 * it asks (`catalogOrder`).
 */
const TICKER = /^[A-Z0-9.^=-]{1,12}$/;

/** Review Recents' default cap, and the queues' default window (D19). */
export const REVIEW_RECENT_LIMIT = 50;
export const REVIEW_MAX_DAYS = 365;
/** Bounds the two review numbers are clamped to: a run of 1–1000, a window of 0 (no limit)–36500 days. */
const REVIEW_LIMIT_RANGE = [1, 1000] as const;
const REVIEW_DAYS_RANGE = [0, 36500] as const;

/** Find Matches' defaults and bounds (plan 35 D8): a window of 1–60 days, a lookback of 0 (no limit)–36500, in transit after 1–365. */
export const MATCH_WINDOW = 10;
export const MATCH_LOOKBACK = 365;
export const MATCH_TRANSIT_DAYS = 10;
const MATCH_WINDOW_RANGE = [1, 60] as const;
const MATCH_LOOKBACK_RANGE = [0, 36500] as const;
const MATCH_TRANSIT_RANGE = [1, 365] as const;

/** A persisted whole number inside `range`, or `fallback` for anything else (a stale blob, a hand edit). */
function wholeIn(value: unknown, range: readonly [number, number], fallback: number): number {
    if (typeof value !== "number" || !Number.isFinite(value)) return fallback;
    return Math.min(range[1], Math.max(range[0], Math.round(value)));
}
function cleanBenchmarks(value: unknown): string[] {
    const picked = Array.isArray(value) ? value.filter((v): v is string => typeof v === "string" && TICKER.test(v)) : [];
    return [...new Set(picked)];
}

const defaults = (): PersistedSettings => ({
    serverUrl: null,
    serverToken: null,
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

/**
 * When the SPA is served in-process by the `ledgeline` binary, that binary
 * injects `window.__LEDGELINE_EMBEDDED__ = true` into the served index.html. In
 * that case the API lives at the SAME ORIGIN as the page, so we force the server
 * URL to the current origin — no setup modal, and immune to a stale/ephemeral
 * port left in localStorage from a previous run. Standalone dev (vite) has no
 * such marker and keeps the null-→-modal flow.
 */
function embeddedServerUrl(): string | null {
    if (typeof window === "undefined") return null;
    return (window as {__LEDGELINE_EMBEDDED__?: boolean}).__LEDGELINE_EMBEDDED__ === true ? window.location.origin : null;
}

/**
 * Why the persisted settings could not be read, when that happened.
 *
 * A corrupt blob throws out of `JSON.parse`, and the catch used to discard
 * everything in it — the verified server URL and every column preference —
 * without a word, so the app reappeared at the first-run setup modal as if it
 * had never been configured and the user had no idea why (FE-5g). Recording it
 * lets the layout say so; the settings themselves still fall back to defaults,
 * because there is genuinely nothing else to fall back to.
 */
let storageError: string | null = null;

function load(): PersistedSettings {
    const embedded = embeddedServerUrl();
    if (typeof localStorage === "undefined") return {...defaults(), serverUrl: embedded ?? null};
    try {
        const raw = localStorage.getItem(SETTINGS_STORAGE_KEY);
        const parsed = raw === null ? ({} as Partial<PersistedSettings>) : (JSON.parse(raw) as Partial<PersistedSettings>);
        return {
            // Embedded mode always wins over any persisted URL.
            serverUrl: embedded ?? (typeof parsed.serverUrl === "string" ? parsed.serverUrl : null),
            serverToken: typeof parsed.serverToken === "string" ? parsed.serverToken : null,
            columns: {...defaultColumns(), ...(typeof parsed.columns === "object" && parsed.columns !== null ? parsed.columns : {})},
            insightsOpen: typeof parsed.insightsOpen === "boolean" ? parsed.insightsOpen : true,
            // Validated against the union rather than merely typechecked as a
            // string: a stale blob naming a tab that no longer exists would
            // otherwise render an empty box with no way back.
            insightsTab: INSIGHTS_TABS.includes(parsed.insightsTab as InsightsTab) ? (parsed.insightsTab as InsightsTab) : "activity",
            hideZeroBalances: typeof parsed.hideZeroBalances === "boolean" ? parsed.hideZeroBalances : true,
            flowsInOpen: typeof parsed.flowsInOpen === "boolean" ? parsed.flowsInOpen : true,
            flowsOutOpen: typeof parsed.flowsOutOpen === "boolean" ? parsed.flowsOutOpen : true,
            netWorthChartOpen: typeof parsed.netWorthChartOpen === "boolean" ? parsed.netWorthChartOpen : true,
            cashFlowChartOpen: typeof parsed.cashFlowChartOpen === "boolean" ? parsed.cashFlowChartOpen : true,
            budgetGapsOpen: typeof parsed.budgetGapsOpen === "boolean" ? parsed.budgetGapsOpen : false,
            startupMoreOpen: typeof parsed.startupMoreOpen === "boolean" ? parsed.startupMoreOpen : false,
            // Validated against the union, like `insightsTab`: a dimension a
            // later version dropped must fall back rather than select nothing.
            holdingsPieDimension: PIE_DIMENSIONS.includes(parsed.holdingsPieDimension as PieDimension)
                ? (parsed.holdingsPieDimension as PieDimension)
                : "holding",
            // Typechecked as a string, like `serverToken` and unlike
            // `insightsTab`: a file id has no closed set to validate against,
            // and the server re-validates its shape on every request anyway.
            // A non-empty string is the only thing worth keeping.
            lastProjectionId: typeof parsed.lastProjectionId === "string" && parsed.lastProjectionId !== "" ? parsed.lastProjectionId : null,
            // Validated against the catalog: a symbol the server no longer
            // offers would otherwise be a checkbox-less request that 400s.
            benchmarks: cleanBenchmarks(parsed.benchmarks),
            // Added with plan 32: a blob saved before then has none of these
            // and gets the defaults; an out-of-range number is clamped.
            reviewRecentLimit: wholeIn(parsed.reviewRecentLimit, REVIEW_LIMIT_RANGE, REVIEW_RECENT_LIMIT),
            reviewMaxDays: wholeIn(parsed.reviewMaxDays, REVIEW_DAYS_RANGE, REVIEW_MAX_DAYS),
            reviewMarkers: typeof parsed.reviewMarkers === "boolean" ? parsed.reviewMarkers : true,
            // Added with plan 35, the same way.
            matchWindow: wholeIn(parsed.matchWindow, MATCH_WINDOW_RANGE, MATCH_WINDOW),
            matchLookback: wholeIn(parsed.matchLookback, MATCH_LOOKBACK_RANGE, MATCH_LOOKBACK),
            matchTransitDays: wholeIn(parsed.matchTransitDays, MATCH_TRANSIT_RANGE, MATCH_TRANSIT_DAYS),
        };
    } catch (cause) {
        storageError = `Saved settings couldn't be read (${cause instanceof Error ? cause.message : String(cause)}) — starting from defaults.`;
        console.warn(`[settings] ${storageError}`, cause);
        return {...defaults(), serverUrl: embedded ?? null};
    }
}

const state = $state<PersistedSettings>(load());

/**
 * Bumped by every SUCCESSFUL `setServerUrl`, including one that verifies the
 * address already stored.
 *
 * The URL alone cannot express "the user just reconnected": the common recovery
 * is the engine restarting on the same port, which leaves it identical. Every
 * refetch guard keyed on the URL therefore sat still and the Reconnect button
 * did nothing (FE-5d).
 */
let serverNonce = $state(0);

function persist(): void {
    if (typeof localStorage === "undefined") return;
    localStorage.setItem(
        SETTINGS_STORAGE_KEY,
        JSON.stringify({
            serverUrl: state.serverUrl,
            serverToken: state.serverToken,
            columns: state.columns,
            insightsOpen: state.insightsOpen,
            insightsTab: state.insightsTab,
            hideZeroBalances: state.hideZeroBalances,
            flowsInOpen: state.flowsInOpen,
            flowsOutOpen: state.flowsOutOpen,
            netWorthChartOpen: state.netWorthChartOpen,
            cashFlowChartOpen: state.cashFlowChartOpen,
            budgetGapsOpen: state.budgetGapsOpen,
            startupMoreOpen: state.startupMoreOpen,
            holdingsPieDimension: state.holdingsPieDimension,
            lastProjectionId: state.lastProjectionId,
            benchmarks: state.benchmarks,
            reviewRecentLimit: state.reviewRecentLimit,
            reviewMaxDays: state.reviewMaxDays,
            reviewMarkers: state.reviewMarkers,
            matchWindow: state.matchWindow,
            matchLookback: state.matchLookback,
            matchTransitDays: state.matchTransitDays,
        })
    );
}

export const settings = {
    /** null until a server URL has been verified — the layout shows the setup modal. */
    get serverUrl(): string | null {
        return state.serverUrl;
    },
    /** Increments on every successful `setServerUrl`; refetch guards key on it so a same-URL reconnect still fires. */
    get serverNonce(): number {
        return serverNonce;
    },
    /** Why the persisted settings were discarded at startup, or null when they loaded (set once, never changes). */
    get storageError(): string | null {
        return storageError;
    },
    get columns(): ColumnConfig {
        return state.columns;
    },
    set columns(columns: ColumnConfig) {
        state.columns = columns;
        persist();
    },
    get insightsOpen(): boolean {
        return state.insightsOpen;
    },
    set insightsOpen(open: boolean) {
        state.insightsOpen = open;
        persist();
    },
    /** Which insights view is showing — see InsightsTab. */
    get insightsTab(): InsightsTab {
        return state.insightsTab;
    },
    set insightsTab(tab: InsightsTab) {
        state.insightsTab = tab;
        persist();
    },
    /** Balances view: hide accounts holding exactly nothing. */
    get hideZeroBalances(): boolean {
        return state.hideZeroBalances;
    },
    set hideZeroBalances(hide: boolean) {
        state.hideZeroBalances = hide;
        persist();
    },
    /** Whether "Money in" is expanded. Also part of what decides the flows are worth fetching. */
    get flowsInOpen(): boolean {
        return state.flowsInOpen;
    },
    set flowsInOpen(open: boolean) {
        state.flowsInOpen = open;
        persist();
    },
    /** Whether "Money out" is expanded. */
    get flowsOutOpen(): boolean {
        return state.flowsOutOpen;
    },
    set flowsOutOpen(open: boolean) {
        state.flowsOutOpen = open;
        persist();
    },
    /** Whether the Net Worth chart is expanded. */
    get netWorthChartOpen(): boolean {
        return state.netWorthChartOpen;
    },
    set netWorthChartOpen(open: boolean) {
        state.netWorthChartOpen = open;
        persist();
    },
    /** Whether the Cash Flow chart is expanded. */
    get cashFlowChartOpen(): boolean {
        return state.cashFlowChartOpen;
    },
    set cashFlowChartOpen(open: boolean) {
        state.cashFlowChartOpen = open;
        persist();
    },
    /** Whether the Budget tab's "not budgeted" section is expanded — and thus whether it is fetched. */
    get budgetGapsOpen(): boolean {
        return state.budgetGapsOpen;
    },
    set budgetGapsOpen(open: boolean) {
        state.budgetGapsOpen = open;
        persist();
    },
    /** Whether Insights' "More metrics" list is expanded. */
    get startupMoreOpen(): boolean {
        return state.startupMoreOpen;
    },
    set startupMoreOpen(open: boolean) {
        state.startupMoreOpen = open;
        persist();
    },
    /** What the Holdings pie is divided by — see the field. */
    get holdingsPieDimension(): PieDimension {
        return state.holdingsPieDimension;
    },
    set holdingsPieDimension(dimension: PieDimension) {
        state.holdingsPieDimension = dimension;
        persist();
    },
    /** The Projections tab's last-loaded scenario file id — see the field. */
    get lastProjectionId(): string | null {
        return state.lastProjectionId;
    },
    set lastProjectionId(id: string | null) {
        state.lastProjectionId = id === null || id === "" ? null : id;
        persist();
    },
    /** The benchmark lines overlaid on the Stocks chart, in catalog order. */
    get benchmarks(): readonly string[] {
        return state.benchmarks;
    },
    /** Tick or untick one benchmark; unknown symbols are ignored. */
    toggleBenchmark(symbol: string, on: boolean): void {
        const others = state.benchmarks.filter((s) => s !== symbol);
        state.benchmarks = cleanBenchmarks(on ? [...others, symbol] : others);
        persist();
    },
    /** Review Recents' cap (D19), 1–1000. */
    get reviewRecentLimit(): number {
        return state.reviewRecentLimit;
    },
    set reviewRecentLimit(limit: number) {
        state.reviewRecentLimit = wholeIn(limit, REVIEW_LIMIT_RANGE, REVIEW_RECENT_LIMIT);
        persist();
    },
    /** The review window in days (D19); 0 = no limit. */
    get reviewMaxDays(): number {
        return state.reviewMaxDays;
    },
    set reviewMaxDays(days: number) {
        state.reviewMaxDays = wholeIn(days, REVIEW_DAYS_RANGE, REVIEW_MAX_DAYS);
        persist();
    },
    /** Whether the journal draws review stripes (D20). */
    get reviewMarkers(): boolean {
        return state.reviewMarkers;
    },
    set reviewMarkers(on: boolean) {
        state.reviewMarkers = on;
        persist();
    },
    /** Find Matches' window in days (plan 35 D8), 1–60. */
    get matchWindow(): number {
        return state.matchWindow;
    },
    set matchWindow(days: number) {
        state.matchWindow = wholeIn(days, MATCH_WINDOW_RANGE, MATCH_WINDOW);
        persist();
    },
    /** Find Matches' lookback in days (D8); 0 = no limit. */
    get matchLookback(): number {
        return state.matchLookback;
    },
    set matchLookback(days: number) {
        state.matchLookback = wholeIn(days, MATCH_LOOKBACK_RANGE, MATCH_LOOKBACK);
        persist();
    },
    /** Days before an unmatched clearing posting reads as "in transit" (D8, D10), 1–365. */
    get matchTransitDays(): number {
        return state.matchTransitDays;
    },
    set matchTransitDays(days: number) {
        state.matchTransitDays = wholeIn(days, MATCH_TRANSIT_RANGE, MATCH_TRANSIT_DAYS);
        persist();
    },
    /** The cross-origin engine token, if one was entered. Null in embedded mode. */
    get serverToken(): string | null {
        return state.serverToken;
    },
    /**
     * Verifies GET /version at `url`; persists only on success. Throws
     * ApiUnreachableError/ApiShapeError.
     *
     * `token` (omit to keep the current one) is the Ledgeline engine's access
     * token, needed only when the engine is at a DIFFERENT origin than this
     * page. It has to be persisted before the probe goes out, because
     * `apiToken()` reads the stored blob rather than this reactive state — so a
     * failed verification rolls it back.
     */
    async setServerUrl(url: string, token?: string): Promise<void> {
        const normalized = url.trim().replace(/\/+$/, "");
        const candidate = token === undefined || token.trim() === "" ? null : token.trim();
        // Verify with the candidate token passed explicitly, so an unreachable
        // server leaves storage completely untouched (a failed attempt must not
        // persist a half-configured server, and must not clobber a working one).
        await new HledgerApi(normalized, candidate ?? undefined).version();
        if (token !== undefined) state.serverToken = candidate;
        state.serverUrl = normalized;
        // AFTER the URL, so anything reacting to the nonce reads the new one.
        // Bumped even when `normalized` equals what was already stored: that is
        // exactly the reconnect-to-a-restarted-engine case, and it is the only
        // signal distinguishing it from no change at all.
        serverNonce += 1;
        persist();
    },
};
