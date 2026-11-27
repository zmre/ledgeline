// Holdings scope + data store (WP-10, now native). The scope (accounts/mode/
// asOf) is Svelte 5 runes state driving the scope bar + URL sync; the report
// and trend are fetched from the native /api/holdings[/series] endpoints for
// the current scope and decoded into the existing domain types, so the holdings
// UI renders unchanged. The scope's asOf defaults to localToday() and is NEVER
// persisted (every fresh visit opens at today). A monotonic request token drops
// stale responses when the scope changes faster than the network answers.
/* eslint-disable svelte/prefer-svelte-reactivity -- the account Sets are
   immutable snapshots: every change replaces `value` wholesale, so plain Set
   is correct (and the contract exposes ReadonlySet). */
import {LedgelineApi} from "#lib/api/native.js";
import {decodeHoldingsReport, decodeHoldingsSeries, decodeOtherHoldingsReport} from "#lib/api/nativeDecode.js";
import type {ISODate} from "#lib/domain/types.js";
import {toggleSubtreeRoot} from "#lib/filters/treeSelect.js";
import {DEFAULT_CHART_VIEW, TAB_ORDER, type ChartView, type HoldingsTab} from "#lib/holdings/params.js";
import type {GainPeriod, HoldingsReport, HoldingsScope, HoldingsSeries, OtherHoldingsReport} from "#lib/holdings/types.js";
import {DEFAULT_GAIN_PERIOD, anchorFor, gainSinceFor, isIntradayWindow, trendWindowFor} from "#lib/holdings/ui/gainPeriod.js";
import {filterParam, sameScope, toggledSelection} from "#lib/holdings/ui/selection.js";
import type {HoldingsUrlState} from "#lib/holdings/ui/urlCodec.js";
import {localToday} from "./filters.svelte";
import type {DataView, LoadStatus} from "./loadState";
import {createResource} from "./resource.svelte";

/**
 * The value-over-time query for `scope`: its accounts, mode and asOf, and the
 * window its GAIN PERIOD names (`trendWindowFor`). One function because three
 * requests must agree on it byte for byte — the Stocks series, the Other
 * series, and the benchmark overlay drawn over the Stocks one, whose points
 * are only meaningful index-aligned to the chart beneath them.
 */
export function trendQuery(scope: HoldingsScope): {
    asOf: ISODate;
    accounts: string;
    mode: HoldingsScope["mode"];
    interval: string;
    since: string;
} {
    const window = trendWindowFor(scope.gainPeriod, scope.asOf);
    return {asOf: scope.asOf, accounts: [...scope.accounts].join(","), mode: scope.mode, ...window};
}

/** Fresh-visit default: everything included, as of today, the last 12 months, no holdings filter (recomputed per call, never remembered). */
export function defaultScope(): HoldingsScope {
    return {accounts: new Set<string>(), mode: "include", asOf: localToday(), gainPeriod: DEFAULT_GAIN_PERIOD, holdings: new Set<string>()};
}

let value = $state<HoldingsScope>(defaultScope());

/**
 * Which sub-tab is open (plans/14). Separate state, NOT a field of
 * `HoldingsScope`: the scope is the report resources' refetch key below, so a
 * tab living in it would refetch the stock report on every tab click.
 *
 * It is still module state rather than the page's own, because the URL sync has
 * to observe it from plain TS — see `subscribeHoldingsUrlState`.
 */
let tab = $state<HoldingsTab>(TAB_ORDER[0]);

/**
 * The tab, as a bindable property so `<HoldingsTabs bind:tab={holdingsTab.value}>`
 * writes straight through to the state the URL sync watches. The scope's
 * setters are named methods because each one preserves an invariant; a tab has
 * none, so an accessor is the honest shape.
 */
export const holdingsTab = {
    get value(): HoldingsTab {
        return tab;
    },
    /**
     * Switching tab clears the holdings filter (plan 47): it names SYMBOLS on
     * Stocks and ACCOUNTS on Other, so carried across it would filter the new
     * tab's chart to rows that cannot exist there.
     */
    set value(next: HoldingsTab) {
        if (next === tab) return;
        tab = next;
        if (value.holdings.size > 0) value = {...value, holdings: new Set<string>()};
    },
};

/**
 * What the Stocks chart plots (plan 47 A1b). Module state beside the tab, and
 * for the same reasons: the URL sync observes it, and it is not part of the
 * scope (both lines come in one series response, so switching fetches
 * nothing). Switching tab keeps it — the Other chart has no Performance view
 * and ignores it, and coming back to Stocks finds the chart as it was left.
 */
let chartView = $state<ChartView>(DEFAULT_CHART_VIEW);

export const holdingsChartView = {
    get value(): ChartView {
        return chartView;
    },
    set value(next: ChartView) {
        chartView = next;
    },
};

/**
 * The scope as `forTab`'s report should be fetched for it: the holdings filter
 * belongs to the tab on screen, so the other tab's report is asked for with
 * none. Applied inside each resource's `load`, so every caller (the page, the
 * global Refresh, Update prices) gets it right without knowing the rule.
 */
function scopeForTab(scope: HoldingsScope, forTab: HoldingsTab): HoldingsScope {
    return tab === forTab || scope.holdings.size === 0 ? scope : {...scope, holdings: new Set<string>()};
}

/** The last scope `holdingsScope.forTab` returned per tab, so an unchanged one keeps its identity. */
const lastForTab: Partial<Record<HoldingsTab, HoldingsScope>> = {};

/**
 * Observe everything the holdings screen mirrors into the query string —
 * scope, tab and chart view — outside component context (used by the holdings URL sync,
 * which is plain TS and cannot declare rune effects itself). The callback fires
 * once immediately, then after every change. Returns an unsubscribe — same
 * contract as subscribeFilters.
 *
 * ONE subscription for both, deliberately: two would mean two debounced writers
 * to the same query string, and whichever fired last would erase the other's
 * keys.
 */
export function subscribeHoldingsUrlState(cb: (s: HoldingsUrlState) => void): () => void {
    return $effect.root(() => {
        $effect(() => {
            cb({scope: value, tab, view: chartView});
        });
    });
}

export const holdingsScope = {
    get value(): HoldingsScope {
        return value;
    },
    /** Toggle an account's selection, keeping the subtree-root invariant (same rules as the journal filters). */
    toggleAccount(name: string): void {
        value = {...value, accounts: toggleSubtreeRoot(value.accounts, name)};
    },
    /** Switch include/exclude; the selection is kept (plans/10 UI behavior). */
    setMode(mode: HoldingsScope["mode"]): void {
        value = {...value, mode};
    },
    setAsOf(asOf: ISODate): void {
        value = {...value, asOf};
    },
    /** Switch the gain window — which also sets the value-over-time chart's window; everything else is kept. */
    setGainPeriod(gainPeriod: GainPeriod): void {
        value = {...value, gainPeriod};
    },
    /** Clear the account selection (back to "everything"); mode, asOf and gain window are kept. */
    clear(): void {
        value = {...value, accounts: new Set<string>()};
    },
    /** Replace the whole scope at once (URL-sync startup restore). */
    replace(s: HoldingsScope): void {
        value = {accounts: new Set(s.accounts), mode: s.mode, asOf: s.asOf, gainPeriod: s.gainPeriod, holdings: new Set(s.holdings)};
    },
    /** Add or remove one row (a symbol on Stocks, an account on Other) from the holdings filter — the dropdown's checkbox. */
    toggleHolding(key: string): void {
        const next = new Set(value.holdings);
        if (!next.delete(key)) next.add(key);
        value = {...value, holdings: next};
    },
    /**
     * Filter to exactly `keys` — a click on a table row, a wedge or a gainer
     * (D7). Clicking what is ALREADY exactly the filter clears it, so the same
     * click undoes itself and the chart goes back to every row.
     */
    selectHoldings(keys: readonly string[]): void {
        value = {...value, holdings: toggledSelection(value.holdings, keys)};
    },
    /** Drop the holdings filter (back to every row); everything else is kept. */
    clearHoldings(): void {
        if (value.holdings.size > 0) value = {...value, holdings: new Set<string>()};
    },
    /**
     * The scope `forTab`'s report is fetched for (see `scopeForTab`), with a
     * STABLE identity: the same object back for as long as it is unchanged. A
     * page effect keyed on it therefore does not re-run when only the other
     * tab's filter moved — clicking a stock must not refetch the Other report.
     */
    forTab(forTab: HoldingsTab): HoldingsScope {
        const next = scopeForTab(value, forTab);
        const last = lastForTab[forTab];
        if (last !== undefined && sameScope(last, next)) return last;
        lastForTab[forTab] = next;
        return next;
    },
};

/**
 * Report and trend are ONE payload, not two.
 *
 * They are fetched together and are only ever meaningful together — a trend
 * from the previous scope drawn under the current scope's table would be a
 * quieter version of exactly the mismatch FE-1 was about. Holding them in one
 * `createResource` value makes that impossible: both land in a single
 * assignment or neither does.
 */
interface HoldingsPayload {
    /** EVERY row in scope — the table, pie and gainers/losers, whatever the holdings filter says (D6). */
    report: HoldingsReport;
    /**
     * The same report narrowed to the holdings filter, for the stat tiles; null
     * when there is no filter (the tiles then read `report`). A second request
     * rather than a sum over the selected rows: totals are the ENGINE's, never
     * recomputed here.
     */
    filtered: HoldingsReport | null;
    /** The chart, narrowed to the holdings filter. */
    trend: HoldingsSeries;
}

/**
 * Whether `scope` reads intraday quotes (`live=1`): a 1wk/1mo window ending
 * today (D10). Always sent when it does — the GET never reaches the network,
 * and with no cached quote the engine answers exactly as without it.
 */
export function liveFor(scope: HoldingsScope): boolean {
    return isIntradayWindow(scope.gainPeriod, scope.asOf, localToday());
}

/** The series request for `scope` on either tab: the window, the y-axis anchor, the filter and the live flag. */
function seriesQuery(scope: HoldingsScope): ReturnType<typeof trendQuery> & {anchor: ReturnType<typeof anchorFor>; holdings?: string; live: boolean} {
    return {...trendQuery(scope), anchor: anchorFor(scope.gainPeriod), holdings: filterParam(scope.holdings), live: liveFor(scope)};
}

const resource = createResource<HoldingsScope, HoldingsPayload>(async (serverUrl, scope) => {
    const api = new LedgelineApi(serverUrl);
    const accounts = [...scope.accounts].join(",");
    // The gain period narrows the report's gain AND sets the chart's window.
    const gainSince = gainSinceFor(scope.gainPeriod, scope.asOf);
    const live = liveFor(scope);
    const holdings = filterParam(scope.holdings);
    const [rawReport, rawFiltered, rawSeries] = await Promise.all([
        api.holdings({asOf: scope.asOf, accounts, mode: scope.mode, gainSince, live}),
        holdings === undefined ? Promise.resolve(null) : api.holdings({asOf: scope.asOf, accounts, mode: scope.mode, gainSince, live, holdings}),
        api.holdingsSeries(seriesQuery(scope)),
    ]);
    return {
        report: decodeHoldingsReport(rawReport),
        filtered: rawFiltered === null ? null : decodeHoldingsReport(rawFiltered),
        trend: decodeHoldingsSeries(rawSeries),
    };
});

export const holdingsData = {
    /** The decoded holdings report for the last loaded scope, or null before the first load. */
    get report(): HoldingsReport | null {
        return resource.value?.report ?? null;
    },
    /** The report narrowed to the holdings filter (the stat tiles), or null when no filter was set. */
    get filtered(): HoldingsReport | null {
        return resource.value?.filtered ?? null;
    },
    /** The value-over-time series for the last loaded scope, or null before the first load. */
    get trend(): HoldingsSeries | null {
        return resource.value?.trend ?? null;
    },
    /** The scope `report` and `trend` were loaded for — what an overlay on the trend must be computed for. */
    get scope(): HoldingsScope | null {
        return resource.query;
    },
    get status(): LoadStatus {
        return resource.status;
    },
    get error(): Error | null {
        return resource.error;
    },
    /** Which branch the page should render — error outranks stale data (FE-5). */
    get view(): DataView {
        return resource.view;
    },
    /**
     * Fetch + decode the report and trend for `scope`; stale responses
     * (superseded by a newer load) are discarded. A holdings filter is sent
     * only while Stocks is the tab on screen (see `scopeForTab`).
     */
    load: (serverUrl: string, scope: HoldingsScope): Promise<void> => resource.load(serverUrl, scopeForTab(scope, "stocks")),
};

/** The Other tab's payload. One value for the same reason `HoldingsPayload` is one — see its comment. */
interface OtherHoldingsPayload {
    report: OtherHoldingsReport;
    trend: HoldingsSeries;
}

/**
 * The Other tab's report + trend, keyed on the SAME scope as the stock report.
 *
 * Its own resource rather than a third field on `HoldingsPayload`, because the
 * two are not fetched together: this one is loaded LAZILY, the first time the
 * Other tab is opened (plans/14), so a user who never clicks it pays nothing.
 * Folding it into the stock payload would make that impossible — `createResource`
 * lands a payload in one assignment or not at all, which is exactly the property
 * we want WITHIN a tab and exactly the wrong one across two.
 *
 * `valueIn` is not sent: the engine values in the journal's own base commodity,
 * which is what the screen wants and what the Stocks tab already shows.
 */
const otherResource = createResource<HoldingsScope, OtherHoldingsPayload>(async (serverUrl, scope) => {
    const api = new LedgelineApi(serverUrl);
    const accounts = [...scope.accounts].join(",");
    // The gain period narrows the change column AND sets the chart's window — the same control, meaning the same thing, on both tabs.
    const gainSince = gainSinceFor(scope.gainPeriod, scope.asOf);
    // The holdings filter narrows the chart only (D9): the table keeps every row.
    const [rawReport, rawSeries] = await Promise.all([
        api.otherHoldings({asOf: scope.asOf, accounts, mode: scope.mode, gainSince, live: liveFor(scope)}),
        api.otherHoldingsSeries(seriesQuery(scope)),
    ]);
    // The series wire shape is the stock series', byte for byte — hence no second decoder.
    return {report: decodeOtherHoldingsReport(rawReport), trend: decodeHoldingsSeries(rawSeries)};
});

export const otherHoldingsData = {
    /** The decoded other-holdings report for the last loaded scope, or null before the first load. */
    get report(): OtherHoldingsReport | null {
        return otherResource.value?.report ?? null;
    },
    /** The value-over-time series for the last loaded scope, or null before the first load. */
    get trend(): HoldingsSeries | null {
        return otherResource.value?.trend ?? null;
    },
    /** The scope `report` and `trend` were loaded for. */
    get scope(): HoldingsScope | null {
        return otherResource.query;
    },
    get status(): LoadStatus {
        return otherResource.status;
    },
    get error(): Error | null {
        return otherResource.error;
    },
    /** Which branch the page should render — error outranks stale data (FE-5). */
    get view(): DataView {
        return otherResource.view;
    },
    /** Fetch + decode the report and trend for `scope`; stale responses are discarded. A filter is sent only while Other is on screen. */
    load: (serverUrl: string, scope: HoldingsScope): Promise<void> => otherResource.load(serverUrl, scopeForTab(scope, "other")),
};
