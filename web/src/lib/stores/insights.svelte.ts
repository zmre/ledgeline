// Insights dashboard data store: fetches /api/insights for the current
// comparison window and decodes it into the InsightsReport domain type. Same
// monotonic-token pattern as reports.svelte.ts — a stale response (superseded
// by a newer load) is dropped, and the last good report stays visible across a
// refetch so only the very first load shows a spinner.

import {LedgelineApi} from "$lib/api/native";
import {decodeInsightsReport} from "$lib/api/nativeDecode";
import type {InsightsReport} from "$lib/reports/insightsTypes";

export type InsightsStatus = "idle" | "loading" | "ready" | "error";

/** The exact query the /api/insights endpoint honors. */
export interface InsightsQueryParams {
    start: string;
    end: string;
    /** Comma-separated cost-of-living exclusion prefixes; omit for the server default. */
    exclude?: string;
}

let report = $state<InsightsReport | null>(null);
let status = $state<InsightsStatus>("idle");
let error = $state<Error | null>(null);
let seq = 0;

export const insights = {
    /** The last successfully decoded report, or null before the first load. */
    get report(): InsightsReport | null {
        return report;
    },
    get status(): InsightsStatus {
        return status;
    },
    get error(): Error | null {
        return error;
    },
    /** Fetch + decode the insights report for `query`; stale responses are discarded. */
    async load(serverUrl: string, query: InsightsQueryParams): Promise<void> {
        const token = ++seq;
        status = "loading";
        try {
            const next = decodeInsightsReport(await new LedgelineApi(serverUrl).insights(query));
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
