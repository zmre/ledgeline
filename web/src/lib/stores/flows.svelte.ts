// Income-statement flow graphs: fetches /api/reports/incomestatement/flows for
// the P&L tab's window and decodes it into the FlowReport domain type. The
// stale-response and payload-tagging behaviour is `createResource`'s. See
// resource.svelte.ts for why both matter.

import {LedgelineApi} from "$lib/api/native";
import {decodeFlowReport} from "$lib/api/nativeDecode";
import type {FlowReport} from "$lib/reports/types";
import {createResource} from "./resource.svelte";
import {settings} from "./settings.svelte";

/** The exact query the flows endpoint honors from this screen. */
export interface FlowsQueryParams {
    from: string;
    to: string;
}

export const flows = createResource<FlowsQueryParams, FlowReport>(async (serverUrl, query) =>
    decodeFlowReport(await new LedgelineApi(serverUrl).incomeStatementFlows(query))
);

/**
 * Fetch the flows only while something is actually looking at them.
 *
 * The diagrams are a SEPARATE endpoint partly so a shut panel costs nothing:
 * building them is a second pass over every posting in the window, on top of
 * the statement's own. So the gate is the P&L tab being open AND at least one
 * of the two panels being expanded.
 *
 * The open flags are read INSIDE the effect rather than passed in, which is
 * what makes expanding a panel after both were shut fetch immediately: the
 * effect is subscribed to them and re-runs on the flip.
 *
 * Must be called during component initialization (it declares an `$effect`).
 */
export function loadFlowsWhenWatched(read: () => {tab: string; query: FlowsQueryParams}): void {
    $effect(() => {
        const {tab, query} = read();
        const serverUrl = settings.serverUrl;
        // Read for its dependency alone: a reconnect usually leaves the URL
        // identical (the engine restarted on the same port), so an effect keyed
        // on the URL never retries after one (FE-5d).
        void settings.serverNonce;
        // Both read unconditionally, never behind `||`: a short-circuited read
        // is not a subscription, so the effect would miss a flip of whichever
        // flag the operator skipped.
        const inOpen = settings.flowsInOpen;
        const outOpen = settings.flowsOutOpen;
        const watched = inOpen || outOpen;
        if (serverUrl === null || tab !== "is" || !watched) return;
        void flows.load(serverUrl, query);
    });
}
