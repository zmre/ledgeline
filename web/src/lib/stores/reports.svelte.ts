// Reports data store: fetches the native /api/reports/{tab} endpoint for the
// active ReportParams and decodes it into the existing SectionedReport/
// PeriodReport domain types, so the WP-07 UI renders unchanged. A monotonic
// request token drops stale responses when params change faster than the
// network answers; the last good report stays visible across a refetch (only
// the very first load shows a spinner).

import {LedgelineApi} from "$lib/api/native";
import {decodePeriodReport, decodeSectionedReport} from "$lib/api/nativeDecode";
import type {PeriodReport, SectionedReport} from "$lib/reports/types";
import type {ReportInterval, ReportParams} from "$lib/reports/ui/params";

export type ReportStatus = "idle" | "loading" | "ready" | "error";

/** The exact query for one tab — only the fields that endpoint honors, so the fetch effect refires minimally. */
export type ReportQuery =
    | {tab: "bs"; asOf: string; depth: number}
    | {tab: "is"; from: string; to: string; depth: number}
    | {tab: "cf"; end: string; interval: ReportInterval; count: number; depth: number}
    | {tab: "nw"; end: string; interval: ReportInterval; count: number};

/** Map ReportParams → the active tab's endpoint query (drives both the fetch and the refetch key). */
export function buildReportQuery(params: ReportParams): ReportQuery {
    switch (params.tab) {
        case "bs":
            return {tab: "bs", asOf: params.asOf, depth: params.depth};
        case "is":
            return {tab: "is", from: params.from, to: params.to, depth: params.depth};
        case "cf":
            return {tab: "cf", end: params.end, interval: params.interval, count: params.count, depth: params.depth};
        case "nw":
            return {tab: "nw", end: params.end, interval: params.interval, count: params.count};
    }
}

async function fetchReport(api: LedgelineApi, query: ReportQuery): Promise<SectionedReport | PeriodReport> {
    switch (query.tab) {
        case "bs":
            return decodeSectionedReport(await api.balanceSheet({asOf: query.asOf, depth: query.depth}));
        case "is":
            return decodeSectionedReport(await api.incomeStatement({from: query.from, to: query.to, depth: query.depth}));
        case "cf":
            return decodePeriodReport(await api.cashFlow({end: query.end, interval: query.interval, count: query.count, depth: query.depth}));
        case "nw":
            return decodePeriodReport(await api.netWorth({end: query.end, interval: query.interval, count: query.count}));
    }
}

let report = $state<SectionedReport | PeriodReport | null>(null);
let status = $state<ReportStatus>("idle");
let error = $state<Error | null>(null);
let seq = 0;

export const reports = {
    /** The last successfully decoded report, or null before the first load. */
    get report(): SectionedReport | PeriodReport | null {
        return report;
    },
    get status(): ReportStatus {
        return status;
    },
    get error(): Error | null {
        return error;
    },
    /** Fetch + decode the report for `query`; stale responses (superseded by a newer load) are discarded. */
    async load(serverUrl: string, query: ReportQuery): Promise<void> {
        const token = ++seq;
        status = "loading";
        try {
            const next = await fetchReport(new LedgelineApi(serverUrl), query);
            if (token !== seq) return;
            report = next;
            status = "ready";
            error = null;
        } catch (cause) {
            if (token !== seq) return;
            status = "error";
            error = cause instanceof Error ? cause : new Error(String(cause));
        }
    },
};
