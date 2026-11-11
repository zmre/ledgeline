// Startup-metrics data store (plans/37): fetches /api/saas for the Insights
// comparison window and decodes it into the SaasReport domain type. Keyed on
// the same `{start, end}` as the insights store; the dashboard fires both loads
// together and renders each part as its data lands. Stale-response handling is
// `createResource`'s (resource.svelte.ts).

import {LedgelineApi} from "#lib/api/native.js";
import {decodeSaasReport} from "#lib/api/nativeDecode.js";
import type {SaasReport} from "#lib/reports/saasTypes.js";
import {createResource} from "./resource.svelte";

/**
 * The query the dashboard sends: the Insights span, plus the browser's local
 * today as `asOf` (as the Subscriptions store does), so "complete month" is
 * judged in the user's timezone rather than the server's. `asOf` is part of
 * the key because it changes the answer; surfaces compare only the SPAN
 * (`sameSpan`) when deciding whether a held report is current.
 */
export interface SaasQueryParams {
    start: string;
    end: string;
    asOf: string;
}

export const saas = createResource<SaasQueryParams, SaasReport>(async (serverUrl, query) => decodeSaasReport(await new LedgelineApi(serverUrl).saas(query)));

/**
 * The same span with the per-customer matrix, for the xlsx export only. Not a
 * resource: the export is one click, never rendered, and must not replace the
 * dashboard's held report.
 */
export async function fetchSaasDetail(serverUrl: string, query: SaasQueryParams): Promise<SaasReport> {
    return decodeSaasReport(await new LedgelineApi(serverUrl).saas({...query, detail: "customers"}));
}
