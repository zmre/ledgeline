// Cash-flow sources: fetches /api/reports/cashflow/sources for the Cash Flow
// tab's window and decodes it into a `PeriodReport` whose rows are the
// counterparty accounts. The stale-response and payload-tagging behaviour is
// `createResource`'s (see resource.svelte.ts).

import {untrack} from "svelte";
import {LedgelineApi} from "$lib/api/native";
import {decodePeriodReport} from "$lib/api/nativeDecode";
import type {PeriodReport} from "$lib/reports/types";
import type {DataView} from "./loadState";
import {sameReportQuery, type ReportQuery} from "./reports.svelte";
import {createResource} from "./resource.svelte";
import {settings} from "./settings.svelte";

/** Exactly the cash flow's own query: the breakdown must answer the table's window. */
export type CashFlowSourcesQuery = Omit<Extract<ReportQuery, {tab: "cf"}>, "tab">;

/** The sources fetch as the chart sees it: a view already gated on the window matching the table's. */
export interface SourcesPanel {
    view: DataView;
    report: PeriodReport | null;
    error: Error | null;
    retry: () => void;
}

export const cashFlowSources = createResource<CashFlowSourcesQuery, PeriodReport>(async (serverUrl, query) =>
    decodePeriodReport(await new LedgelineApi(serverUrl).cashFlowSources(query))
);

/** Whether a held breakdown answers exactly `query` — anything else must not be drawn over this table. */
export function sourcesMatch(held: CashFlowSourcesQuery | null, query: CashFlowSourcesQuery): boolean {
    return held !== null && sameReportQuery({tab: "cf", ...held}, {tab: "cf", ...query});
}

/**
 * Fetch the breakdown only while something is looking at it.
 *
 * It is a separate endpoint so that costs nothing otherwise: a second pass over
 * every posting in the window. The gate is the Cash Flow tab being open, the
 * chart panel expanded, AND the chart showing "By source" — "By account" draws
 * the table's own report and needs no request.
 *
 * Every flag is read inside the effect, unconditionally, so flipping any of
 * them re-runs it (a short-circuited read is not a subscription).
 *
 * A re-run that asks for what was already requested on this connection — the
 * panel re-expanded, the mode toggled back, the tab revisited — sends nothing:
 * the held (or in-flight) answer is still the answer. Only a failed request is
 * asked again. A fresh mount of the page starts with nothing requested, so it
 * refetches as the table does. `serverNonce` is the invalidation, exactly as for the report
 * table this breakdown sits above: a reconnect moves it, and the next look
 * refetches.
 *
 * Must be called during component initialization (it declares an `$effect`).
 */
export function loadSourcesWhenWatched(read: () => {tab: string; query: CashFlowSourcesQuery}): void {
    // What was last requested for: server, connection (`serverNonce`) and window.
    // Per call (so per page mount), like the report table's own load: leaving the
    // page and coming back refetches, which is what picks up a journal edit.
    // Plain, not `$state`: written by the effect that reads it.
    let requested: {serverUrl: string; nonce: number; query: CashFlowSourcesQuery} | null = null;
    $effect(() => {
        const {tab, query} = read();
        const serverUrl = settings.serverUrl;
        // The nonce, not just the URL: a reconnect usually leaves the URL
        // identical, so an effect keyed on the URL never retries after one (FE-5d).
        const nonce = settings.serverNonce;
        const open = settings.cashFlowChartOpen;
        const mode = settings.cashFlowChartMode;
        if (serverUrl === null || tab !== "cf" || !open || mode !== "source") return;
        const same = requested !== null && requested.serverUrl === serverUrl && requested.nonce === nonce && sourcesMatch(requested.query, query);
        // Untracked: a request settling must not re-run this, or a persistent
        // failure would retry itself in a loop.
        if (same && untrack(() => cashFlowSources.status) !== "error") return;
        requested = {serverUrl, nonce, query};
        void cashFlowSources.load(serverUrl, query);
    });
}
