// Insights dashboard data store: fetches /api/insights for the current
// comparison window and decodes it into the InsightsReport domain type. The
// stale-response and payload-tagging behaviour is `createResource`'s — see
// resource.svelte.ts for why both matter.

import {LedgelineApi} from "$lib/api/native";
import {decodeInsightsReport} from "$lib/api/nativeDecode";
import type {InsightsReport} from "$lib/reports/insightsTypes";
import {createResource} from "./resource.svelte";

/** The exact query the /api/insights endpoint honors. */
export interface InsightsQueryParams {
    start: string;
    end: string;
    /** Comma-separated cost-of-living exclusion prefixes; omit for the server default. */
    exclude?: string;
}

export const insights = createResource<InsightsQueryParams, InsightsReport>(async (serverUrl, query) =>
    decodeInsightsReport(await new LedgelineApi(serverUrl).insights(query))
);
