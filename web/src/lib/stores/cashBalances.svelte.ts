// Balances-view data store: `/api/balances/cash` decoded into the
// CashBalancesView domain type. Stale-response handling and payload tagging are
// `createResource`'s — see resource.svelte.ts.
//
// The query carries the journal GENERATION as well as the date: the balances
// are a pure function of the two, and keying on the generation is what makes
// an edit (or a file change the poll notices) refetch them.

import {LedgelineApi} from "#lib/api/native.js";
import {decodeCashBalances} from "#lib/api/journalViewDecode.js";
import type {CashBalancesView} from "#lib/journalView/types.js";
import {createResource} from "./resource.svelte";

export interface CashBalancesQuery {
    /** Inclusive as-of date: the browser's local today. */
    asOf: string;
    /** The `/api/journal` generation the answer must describe. */
    generation: string;
}

export const cashBalances = createResource<CashBalancesQuery, CashBalancesView>(async (serverUrl, query) =>
    decodeCashBalances(await new LedgelineApi(serverUrl).cashBalances(query.asOf))
);
