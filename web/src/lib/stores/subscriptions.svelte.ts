// Subscriptions data store: fetches /api/subscriptions and decodes it into the
// SubscriptionsReport domain type. The stale-response and payload-tagging
// behaviour is `createResource`'s — see resource.svelte.ts.
//
// Unlike the report/insights stores this takes no period: detection always
// scans a trailing window ending at `asOf`, which the caller supplies from the
// browser's local clock so the window is deterministic and in the user's own
// timezone (rather than the server's UTC today).

import {LedgelineApi} from "$lib/api/native";
import {decodeSubscriptionsReport} from "$lib/api/nativeDecode";
import type {SubscriptionsReport} from "$lib/reports/insightsTypes";
import {createResource} from "./resource.svelte";

/** `load(serverUrl, asOf)` — the query is the trailing window's end date. */
export const subscriptions = createResource<string, SubscriptionsReport>(async (serverUrl, asOf) =>
    decodeSubscriptionsReport(await new LedgelineApi(serverUrl).subscriptions({asOf}))
);
