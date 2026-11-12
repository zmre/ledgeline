// Settings store (WP-02): Svelte 5 runes state.
//
// Two halves, kept in two places (plan 38 WP-5):
//
//  - The CONNECTION — `serverUrl`, `serverToken` — describes this client, so it
//    stays in localStorage under a versioned key. `setServerUrl` verifies
//    GET /version before persisting.
//  - The VIEW STATE — columns, open panels, review and matching numbers,
//    benchmarks, … (`UiPrefs`) — belongs to the ledger, and lives on the
//    engine (`GET`/`PATCH /api/ui-prefs`): `ledgeline.toml` beside the journal,
//    over the global `ui-prefs.toml`, over the defaults. The desktop app's
//    WebView origin changes every launch, so localStorage forgot all of it.
//
// Until `loadUiPrefs` hears from the engine — and for good when the engine says
// it leaves them to the browser (`storage: "browser"`: the e2e harness, or an
// engine without the route) — the view state is read from and saved to the
// localStorage blob exactly as before WP-5 ("local" mode). Once the engine
// keeps them ("server" mode), every change is PATCHed, debounced, naming only
// the keys that changed, and localStorage holds only the connection.

import {HledgerApi, SETTINGS_STORAGE_KEY} from "#lib/api/client.js";
import {LedgelineApi} from "#lib/api/native.js";
import {decodeUiPrefsState} from "#lib/api/nativeDecode.js";
import type {PieDimension} from "#lib/holdings/profileTypes.js";
import {
    cleanBenchmarks,
    cleanUiPrefs,
    COLUMN_KEYS,
    defaultUiPrefs,
    legacyUiPatch,
    MATCH_LOOKBACK,
    MATCH_LOOKBACK_RANGE,
    MATCH_TRANSIT_DAYS,
    MATCH_TRANSIT_RANGE,
    MATCH_WINDOW,
    MATCH_WINDOW_RANGE,
    mergePatch,
    REVIEW_DAYS_RANGE,
    REVIEW_LIMIT_RANGE,
    REVIEW_MAX_DAYS,
    REVIEW_RECENT_LIMIT,
    UI_PREF_KEYS,
    wholeIn,
    withoutUiFields,
    type ColumnConfig,
    type InsightsTab,
    type UiPrefs,
    type UiPrefsIssue,
    type UiPrefsPatch,
} from "#lib/settings/uiPrefs.js";

export type {ColumnConfig, InsightsTab} from "#lib/settings/uiPrefs.js";
export {MATCH_LOOKBACK, MATCH_TRANSIT_DAYS, MATCH_WINDOW, REVIEW_MAX_DAYS, REVIEW_RECENT_LIMIT} from "#lib/settings/uiPrefs.js";

interface ConnectionSettings {
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
}

type Settings = ConnectionSettings & UiPrefs;

/** How long a save waits for further changes before it is sent (plan 38 WP-5). */
export const UI_PREFS_SAVE_DELAY_MS = 500;

/**
 * How long the first `GET /api/ui-prefs` may take before the app gives up on
 * it and renders with what it has. The engine answers it from two small files.
 */
const UI_PREFS_LOAD_TIMEOUT_MS = 5_000;

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

/** The localStorage blob, parsed; `{}` when there is none or it cannot be read. */
function readBlob(): Record<string, unknown> {
    if (typeof localStorage === "undefined") return {};
    try {
        const raw = localStorage.getItem(SETTINGS_STORAGE_KEY);
        const parsed: unknown = raw === null ? {} : JSON.parse(raw);
        return typeof parsed === "object" && parsed !== null && !Array.isArray(parsed) ? (parsed as Record<string, unknown>) : {};
    } catch {
        return {};
    }
}

function load(): Settings {
    const embedded = embeddedServerUrl();
    if (typeof localStorage === "undefined") return {...defaultUiPrefs(), serverUrl: embedded ?? null, serverToken: null};
    try {
        const raw = localStorage.getItem(SETTINGS_STORAGE_KEY);
        const parsed = raw === null ? ({} as Record<string, unknown>) : (JSON.parse(raw) as Record<string, unknown>);
        return {
            // Embedded mode always wins over any persisted URL.
            serverUrl: embedded ?? (typeof parsed.serverUrl === "string" ? parsed.serverUrl : null),
            serverToken: typeof parsed.serverToken === "string" ? parsed.serverToken : null,
            // The blob's view state, for the first paint and for "local" mode.
            ...cleanUiPrefs(parsed),
        };
    } catch (cause) {
        storageError = `Saved settings couldn't be read (${cause instanceof Error ? cause.message : String(cause)}) — starting from defaults.`;
        console.warn(`[settings] ${storageError}`, cause);
        return {...defaultUiPrefs(), serverUrl: embedded ?? null, serverToken: null};
    }
}

const state = $state<Settings>(load());

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

/**
 * Where the view state is saved: "local" (the localStorage blob, as before
 * WP-5) until the engine says it keeps it, then "server". See the file header.
 */
let uiMode: "local" | "server" = "local";
/** True once the first `loadUiPrefs` has settled, either way. */
let uiReady = $state(false);
/** What the engine reported about its settings files, plus any failed save. */
let uiNotices = $state<readonly string[]>([]);
/** Keys changed since the last save went out, and the engine to send them to. */
let pending: UiPrefsPatch = {};
let pendingTimer: ReturnType<typeof setTimeout> | null = null;
let uiServerUrl: string | null = null;

function writeBlob(blob: Record<string, unknown>): void {
    if (typeof localStorage === "undefined") return;
    localStorage.setItem(SETTINGS_STORAGE_KEY, JSON.stringify(blob));
}

/**
 * Persist to localStorage: the connection always; the view state too in
 * "local" mode. In "server" mode anything else in the blob is left as it is —
 * an old blob's view state is the one-time migration's to consume, and it
 * consumes it only for a ledger with no UI preferences of its own.
 */
function persist(): void {
    if (typeof localStorage === "undefined") return;
    const connection = {serverUrl: state.serverUrl, serverToken: state.serverToken};
    if (uiMode === "server") {
        writeBlob({...readBlob(), ...connection});
        return;
    }
    writeBlob({
        ...connection,
        ...Object.fromEntries(UI_PREF_KEYS.map((key) => [key, state[key]])),
    });
}

/** Record a change to the view state where this session keeps it. */
function save(patch: UiPrefsPatch): void {
    if (uiMode === "local") {
        persist();
        return;
    }
    pending = mergePatch(pending, patch);
    if (pendingTimer !== null) clearTimeout(pendingTimer);
    pendingTimer = setTimeout(() => void flushUiPrefs(), UI_PREFS_SAVE_DELAY_MS);
}

/** Human-readable lines for the engine's issues: `ledgeline.toml: ui.matchWindow must be a whole number`. */
function describeIssues(issues: readonly UiPrefsIssue[]): string[] {
    return issues.map((issue) => `${issue.file}: ${issue.key === null ? "" : `${issue.key} `}${issue.message}`);
}

/** Send the pending save now. Resolves once it has been answered (or has failed, which is reported, not thrown). */
async function flushUiPrefs(): Promise<void> {
    if (pendingTimer !== null) clearTimeout(pendingTimer);
    pendingTimer = null;
    const body = pending;
    pending = {};
    if (uiServerUrl === null || Object.keys(body).length === 0) return;
    try {
        // The answer is NOT applied to the state: the user may have changed
        // more since this went out, and applying it would undo that.
        const answer = decodeUiPrefsState(await new LedgelineApi(uiServerUrl).patchUiPrefs(body));
        uiNotices = describeIssues(answer.issues);
    } catch (cause) {
        const message = cause instanceof Error ? cause.message : String(cause);
        uiNotices = [...uiNotices.filter((n) => !n.startsWith("Couldn't save")), `Couldn't save view settings: ${message}`];
        console.warn("[settings] saving UI preferences failed", cause);
    }
}

function applyUiPrefs(prefs: UiPrefs): void {
    Object.assign(state, prefs);
}

/**
 * Fetch the view state from the engine at `serverUrl` and switch to saving it
 * there; migrate an old localStorage blob's view state into a ledger that has
 * none yet, once. Never throws: an engine that cannot answer leaves the
 * session in "local" mode.
 */
async function loadUiPrefs(serverUrl: string): Promise<void> {
    // A save still waiting for the previous engine goes there first.
    if (uiMode === "server") await flushUiPrefs();
    const api = new LedgelineApi(serverUrl, {timeoutMs: UI_PREFS_LOAD_TIMEOUT_MS});
    try {
        let answer = decodeUiPrefsState(await api.getUiPrefs());
        if (answer.storage === "browser") {
            uiMode = "local";
            return;
        }
        const blob = readBlob();
        const legacy = legacyUiPatch(blob);
        let migrated = false;
        if (Object.keys(legacy).length > 0 && !answer.ledgerHasUi) {
            try {
                answer = decodeUiPrefsState(await api.patchUiPrefs(legacy));
                migrated = true;
            } catch (cause) {
                // Keep the blob for next time, and its values for this session.
                console.warn("[settings] could not move saved view settings to the ledger; keeping them in this browser", cause);
                answer = {...answer, prefs: cleanUiPrefs({...answer.prefs, ...blob})};
            }
        }
        applyUiPrefs(answer.prefs);
        uiNotices = describeIssues(answer.issues);
        uiServerUrl = serverUrl;
        uiMode = "server";
        if (migrated) writeBlob(withoutUiFields(readBlob()));
    } catch (cause) {
        // No route (hledger-web, an older engine) or no answer: the browser's
        // copy is all there is, as before WP-5.
        console.warn("[settings] UI preferences unavailable from the engine; using this browser's", cause);
        uiMode = "local";
    } finally {
        uiReady = true;
    }
}

/** The members of `next` that differ from `previous`. */
function changedColumns(previous: ColumnConfig, next: ColumnConfig): Partial<ColumnConfig> {
    return Object.fromEntries(COLUMN_KEYS.filter((key) => previous[key] !== next[key]).map((key) => [key, next[key]]));
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
    /** Whether the first view-state load from the engine has settled (the layout holds the page until it has). */
    get uiPrefsReady(): boolean {
        return uiReady;
    },
    /** What the engine said about its settings files, and any save that failed; empty when all is well. */
    get uiPrefsNotices(): readonly string[] {
        return uiNotices;
    },
    /** Load the view state from the engine at `serverUrl` (see `loadUiPrefs`). */
    loadUiPrefs,
    /** Send any debounced view-state save now. */
    flushUiPrefs,
    get columns(): ColumnConfig {
        return state.columns;
    },
    set columns(columns: ColumnConfig) {
        const changed = changedColumns(state.columns, columns);
        state.columns = columns;
        save({columns: changed});
    },
    get insightsOpen(): boolean {
        return state.insightsOpen;
    },
    set insightsOpen(open: boolean) {
        state.insightsOpen = open;
        save({insightsOpen: open});
    },
    /** Which insights view is showing — see InsightsTab. */
    get insightsTab(): InsightsTab {
        return state.insightsTab;
    },
    set insightsTab(tab: InsightsTab) {
        state.insightsTab = tab;
        save({insightsTab: tab});
    },
    /** Balances view: hide accounts holding exactly nothing. */
    get hideZeroBalances(): boolean {
        return state.hideZeroBalances;
    },
    set hideZeroBalances(hide: boolean) {
        state.hideZeroBalances = hide;
        save({hideZeroBalances: hide});
    },
    /** Whether "Money in" is expanded. Also part of what decides the flows are worth fetching. */
    get flowsInOpen(): boolean {
        return state.flowsInOpen;
    },
    set flowsInOpen(open: boolean) {
        state.flowsInOpen = open;
        save({flowsInOpen: open});
    },
    /** Whether "Money out" is expanded. */
    get flowsOutOpen(): boolean {
        return state.flowsOutOpen;
    },
    set flowsOutOpen(open: boolean) {
        state.flowsOutOpen = open;
        save({flowsOutOpen: open});
    },
    /** Whether the Net Worth chart is expanded. */
    get netWorthChartOpen(): boolean {
        return state.netWorthChartOpen;
    },
    set netWorthChartOpen(open: boolean) {
        state.netWorthChartOpen = open;
        save({netWorthChartOpen: open});
    },
    /** Whether the Cash Flow chart is expanded. */
    get cashFlowChartOpen(): boolean {
        return state.cashFlowChartOpen;
    },
    set cashFlowChartOpen(open: boolean) {
        state.cashFlowChartOpen = open;
        save({cashFlowChartOpen: open});
    },
    /** Whether the Budget tab's "not budgeted" section is expanded — and thus whether it is fetched. */
    get budgetGapsOpen(): boolean {
        return state.budgetGapsOpen;
    },
    set budgetGapsOpen(open: boolean) {
        state.budgetGapsOpen = open;
        save({budgetGapsOpen: open});
    },
    /** Whether Insights' "More metrics" list is expanded. */
    get startupMoreOpen(): boolean {
        return state.startupMoreOpen;
    },
    set startupMoreOpen(open: boolean) {
        state.startupMoreOpen = open;
        save({startupMoreOpen: open});
    },
    /** What the Holdings pie is divided by — see the field. */
    get holdingsPieDimension(): PieDimension {
        return state.holdingsPieDimension;
    },
    set holdingsPieDimension(dimension: PieDimension) {
        state.holdingsPieDimension = dimension;
        save({holdingsPieDimension: dimension});
    },
    /** The Projections tab's last-loaded scenario file id — see the field. */
    get lastProjectionId(): string | null {
        return state.lastProjectionId;
    },
    set lastProjectionId(id: string | null) {
        state.lastProjectionId = id === null || id === "" ? null : id;
        save({lastProjectionId: state.lastProjectionId});
    },
    /** The benchmark lines overlaid on the Stocks chart, in catalog order. */
    get benchmarks(): readonly string[] {
        return state.benchmarks;
    },
    /** Tick or untick one benchmark; unknown symbols are ignored. */
    toggleBenchmark(symbol: string, on: boolean): void {
        const others = state.benchmarks.filter((s) => s !== symbol);
        state.benchmarks = cleanBenchmarks(on ? [...others, symbol] : others);
        save({benchmarks: [...state.benchmarks]});
    },
    /** Review Recents' cap (D19), 1–1000. */
    get reviewRecentLimit(): number {
        return state.reviewRecentLimit;
    },
    set reviewRecentLimit(limit: number) {
        state.reviewRecentLimit = wholeIn(limit, REVIEW_LIMIT_RANGE, REVIEW_RECENT_LIMIT);
        save({reviewRecentLimit: state.reviewRecentLimit});
    },
    /** The review window in days (D19); 0 = no limit. */
    get reviewMaxDays(): number {
        return state.reviewMaxDays;
    },
    set reviewMaxDays(days: number) {
        state.reviewMaxDays = wholeIn(days, REVIEW_DAYS_RANGE, REVIEW_MAX_DAYS);
        save({reviewMaxDays: state.reviewMaxDays});
    },
    /** Whether the journal draws review stripes (D20). */
    get reviewMarkers(): boolean {
        return state.reviewMarkers;
    },
    set reviewMarkers(on: boolean) {
        state.reviewMarkers = on;
        save({reviewMarkers: on});
    },
    /** Find Matches' window in days (plan 35 D8), 1–60. */
    get matchWindow(): number {
        return state.matchWindow;
    },
    set matchWindow(days: number) {
        state.matchWindow = wholeIn(days, MATCH_WINDOW_RANGE, MATCH_WINDOW);
        save({matchWindow: state.matchWindow});
    },
    /** Find Matches' lookback in days (D8); 0 = no limit. */
    get matchLookback(): number {
        return state.matchLookback;
    },
    set matchLookback(days: number) {
        state.matchLookback = wholeIn(days, MATCH_LOOKBACK_RANGE, MATCH_LOOKBACK);
        save({matchLookback: state.matchLookback});
    },
    /** Days before an unmatched clearing posting reads as "in transit" (D8, D10), 1–365. */
    get matchTransitDays(): number {
        return state.matchTransitDays;
    },
    set matchTransitDays(days: number) {
        state.matchTransitDays = wholeIn(days, MATCH_TRANSIT_RANGE, MATCH_TRANSIT_DAYS);
        save({matchTransitDays: state.matchTransitDays});
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
