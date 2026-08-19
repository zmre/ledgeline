// Reports data store: fetches the native /api/reports/{tab} endpoint for the
// active ReportParams and decodes it into the existing SectionedReport/
// PeriodReport domain types, so the WP-07 UI renders unchanged. A monotonic
// request token drops stale responses when params change faster than the
// network answers; the last good report stays visible across a refetch (only
// the very first load shows a spinner).

import {LedgelineApi} from "$lib/api/native";
import {decodeBalanceSheetReport, decodeBudgetReport, decodePeriodReport, decodeSectionedReport} from "$lib/api/nativeDecode";
import type {ISODate} from "$lib/domain/types";
import {monthsBetween} from "$lib/reports/periods";
import type {BalanceSheetReport, BudgetReport, PeriodReport, SectionedReport} from "$lib/reports/types";
import type {ReportInterval, ReportParams} from "$lib/reports/ui/params";
import type {LoadStatus} from "./loadState";
import {createResource} from "./resource.svelte";

/** The union of every report shape the store can hold. */
export type AnyReport = SectionedReport | PeriodReport | BudgetReport | BalanceSheetReport;

/** The exact query for one tab — only the fields that endpoint honors, so the fetch effect refires minimally. */
export type ReportQuery =
    // Insights and Subscriptions are served by their own stores
    // (insights.svelte.ts / subscriptions.svelte.ts); these inert variants only
    // keep the tab switches exhaustive — neither is ever fetched here.
    | {tab: "insights"}
    | {tab: "subs"}
    | {tab: "bs"; asOf: string}
    | {tab: "is"; from: string; to: string; depth: number}
    | {tab: "cf"; end: string; interval: ReportInterval; count: number; depth: number}
    | {tab: "nw"; end: string; interval: ReportInterval; count: number; depth: number}
    | {tab: "budget"; from: string; end: string; count: number; depth: number};

/** Map ReportParams → the active tab's endpoint query (drives both the fetch and the refetch key). */
export function buildReportQuery(params: ReportParams): ReportQuery {
    switch (params.tab) {
        case "insights":
            return {tab: "insights"};
        case "subs":
            return {tab: "subs"};
        case "bs":
            return {tab: "bs", asOf: params.asOf};
        case "is":
            return {tab: "is", from: params.from, to: params.to, depth: params.depth};
        case "cf":
            return {tab: "cf", end: params.end, interval: params.interval, count: params.count, depth: params.depth};
        case "nw":
            return {tab: "nw", end: params.end, interval: params.interval, count: params.count, depth: params.depth};
        case "budget": {
            // The budget summary spans the from/to range as monthly buckets, aggregated client-side.
            // `from` is redundant for the fetch (end + count already describe the span) but is carried
            // so `sameReportQuery` can tell one span from another: the export names the file after
            // params.from, and two different `from` dates can round to the same month count.
            const span = budgetSpan(params.from, params.to);
            return {tab: "budget", from: params.from, end: span.to, count: span.count, depth: params.depth};
        }
    }
}

/**
 * The span the budget bars ACTUALLY cover, given the controls' `from`/`to`.
 *
 * The engine takes `{end, count}` and walks whole months backwards, so the first
 * bucket always starts on the 1st — `from`'s day-of-month is discarded. It does
 * truncate the last bucket at `end`, so only the START drifts. With
 * `from = 2026-01-15` the bar therefore includes 2026-01-01…01-14, which a
 * drill-down filtered to the raw controls excludes: measured $720.00 in the bar
 * against $20.00 in the journal it links to.
 *
 * The bars are the thing that cannot move — a monthly goal is a whole-month
 * figure, so comparing it to a partial month is what the envelope view is for.
 * So the drill-down is widened to this span instead, and `BudgetSummary` shows
 * it, rather than silently filtering to a narrower set than it charted.
 */
export function budgetSpan(from: ISODate, to: ISODate): {from: ISODate; to: ISODate; count: number} {
    return {from: `${from.slice(0, 7)}-01`, to, count: monthsBetween(from, to)};
}

/**
 * Whether two queries ask for exactly the same report.
 *
 * Used to gate the export: `exportInfo` names and titles the workbook from the
 * CURRENT controls while the workbook itself is built from the report the store
 * is holding, so those two may only ever be combined when they are the same
 * request (FE-1). Both are flat records of primitives.
 */
export function sameReportQuery(a: ReportQuery, b: ReportQuery): boolean {
    const left = a as Record<string, unknown>;
    const right = b as Record<string, unknown>;
    const keys = Object.keys(left);
    if (keys.length !== Object.keys(right).length) return false;
    return keys.every((key) => left[key] === right[key]);
}

async function fetchReport(api: LedgelineApi, query: ReportQuery): Promise<AnyReport> {
    switch (query.tab) {
        case "insights":
        case "subs":
            // Unreachable: the reports page loads these from their own stores and
            // never calls this one for those tabs.
            throw new Error(`${query.tab} is served by its own store`);
        case "bs":
            // The GROUPED endpoint (plans/12): three boxes, market-valued, one
            // number per line. `/api/reports/balancesheet` still exists and is
            // still golden-tested — it is just no longer what the tab renders.
            //
            // No `depth`: absent means UNCLAMPED on this endpoint, which is what
            // expanding a group has to show. It cannot be spelled as a number —
            // `depth=0` is already hledger's totals-only.
            return decodeBalanceSheetReport(await api.balanceSheetGrouped({asOf: query.asOf}));
        case "is":
            return decodeSectionedReport(await api.incomeStatement({from: query.from, to: query.to, depth: query.depth}));
        case "cf":
            return decodePeriodReport(await api.cashFlow({end: query.end, interval: query.interval, count: query.count, depth: query.depth}));
        case "nw":
            return decodePeriodReport(await api.netWorth({end: query.end, interval: query.interval, count: query.count, depth: query.depth}));
        case "budget":
            return decodeBudgetReport(await api.budget({end: query.end, interval: "monthly", count: query.count, depth: query.depth}));
    }
}

/**
 * The report store: the held report AND the query that produced it, kept as one
 * value by `createResource` so they can never drift apart.
 *
 * `bs`/`is` are both `SectionedReport` and `cf`/`nw` are both `PeriodReport`,
 * so the report's own shape cannot say which tab it belongs to and no type
 * error is possible when they are mixed up. The page used to pick a renderer by
 * shape alone: a balance sheet that was already loaded stayed on screen — and
 * in the export — when the P&L tab's fetch failed, under the P&L's label
 * (FE-1). Tagging the payload with its query is what lets the page refuse, and
 * `reports.query` is how it asks.
 *
 * `report` is the historical name for the payload; the underlying resource
 * calls it `value`.
 */
const resource = createResource<ReportQuery, AnyReport>((serverUrl, query) => fetchReport(new LedgelineApi(serverUrl), query));

export const reports = {
    /** The last successfully decoded report, or null before the first load. */
    get report(): AnyReport | null {
        return resource.value;
    },
    /** The query `report` came from — compare against the live one before rendering or exporting it. */
    get query(): ReportQuery | null {
        return resource.query;
    },
    get status(): LoadStatus {
        return resource.status;
    },
    get error(): Error | null {
        return resource.error;
    },
    /** Fetch + decode the report for `query`; stale responses (superseded by a newer load) are discarded. */
    load: resource.load,
};
