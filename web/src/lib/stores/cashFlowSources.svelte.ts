// Cash-flow sources: fetches /api/reports/cashflow/sources for the Cash Flow
// tab's window and decodes it into a `PeriodReport` whose rows are the
// counterparty accounts. The stale-response and payload-tagging behaviour is
// `createResource`'s (see resource.svelte.ts).

import {LedgelineApi} from "$lib/api/native";
import {decodePeriodReport} from "$lib/api/nativeDecode";
import type {PeriodReport} from "$lib/reports/types";
import type {ReportInterval} from "$lib/reports/ui/params";
import type {DataView} from "./loadState";
import {createResource} from "./resource.svelte";
import {settings} from "./settings.svelte";

/** Exactly the cash flow's own query: the breakdown must answer the table's window. */
export interface CashFlowSourcesQuery {
    end: string;
    interval: ReportInterval;
    count: number;
    depth: number;
}

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
    return held !== null && held.end === query.end && held.interval === query.interval && held.count === query.count && held.depth === query.depth;
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
 * Must be called during component initialization (it declares an `$effect`).
 */
export function loadSourcesWhenWatched(read: () => {tab: string; query: CashFlowSourcesQuery}): void {
    $effect(() => {
        const {tab, query} = read();
        const serverUrl = settings.serverUrl;
        // Read for its dependency alone: a reconnect usually leaves the URL
        // identical, so an effect keyed on the URL never retries after one (FE-5d).
        void settings.serverNonce;
        const open = settings.cashFlowChartOpen;
        const mode = settings.cashFlowChartMode;
        if (serverUrl === null || tab !== "cf" || !open || mode !== "source") return;
        void cashFlowSources.load(serverUrl, query);
    });
}
