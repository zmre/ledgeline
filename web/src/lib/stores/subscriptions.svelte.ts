// Subscriptions data store: fetches /api/subscriptions and decodes it into the
// SubscriptionsReport domain type. Same monotonic-token pattern as the other
// report stores — a stale response (superseded by a newer load) is dropped, and
// the last good report stays visible across a refetch.
//
// Unlike the report/insights stores this takes no period: detection always
// scans a trailing window ending at `asOf`, which the caller supplies from the
// browser's local clock so the window is deterministic and in the user's own
// timezone (rather than the server's UTC today).

import {LedgelineApi} from "$lib/api/native";
import {decodeSubscriptionsReport} from "$lib/api/nativeDecode";
import type {SubscriptionsReport} from "$lib/reports/insightsTypes";

export type SubscriptionsStatus = "idle" | "loading" | "ready" | "error";

let report = $state<SubscriptionsReport | null>(null);
let status = $state<SubscriptionsStatus>("idle");
let error = $state<Error | null>(null);
let seq = 0;

export const subscriptions = {
    /** The last successfully decoded report, or null before the first load. */
    get report(): SubscriptionsReport | null {
        return report;
    },
    get status(): SubscriptionsStatus {
        return status;
    },
    get error(): Error | null {
        return error;
    },
    /** Fetch + decode the trailing-window report ending at `asOf`. */
    async load(serverUrl: string, asOf: string): Promise<void> {
        const token = ++seq;
        status = "loading";
        try {
            const next = decodeSubscriptionsReport(await new LedgelineApi(serverUrl).subscriptions({asOf}));
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
