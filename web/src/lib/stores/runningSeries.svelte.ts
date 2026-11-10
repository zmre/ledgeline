// The pinned account's balance over time (plan 36 amendment A1):
// `/api/journal/running/series` decoded into `BalanceSeries`, the line the
// Balances tab draws in place of its pie while an account is pinned.
// Stale-response handling is `createResource`'s.
//
// Keyed on (account, commodity, from, to, today, generation). Unlike the
// Balances LIST, the line follows the journal filter's DATE range — it
// describes the rows on screen, so "narrow to gap" zooms the chart with the
// table — but not its accounts or search: the values are the whole journal's,
// and the range only sets the window.
//
// Fetched only while the Balances tab is open with a pin set (InsightsPanel),
// the same rule `cashBalances` follows: the tab is the only reader.

import {LedgelineApi} from "#lib/api/native.js";
import {decodeBalanceSeries} from "#lib/api/journalViewDecode.js";
import type {BalanceSeries} from "#lib/journalView/types.js";
import {createResource} from "./resource.svelte";

export interface RunningSeriesQuery {
    /** The pinned subtree root. */
    account: string;
    /** The pin's commodity; null is the engine's default, as for the Balance column. */
    commodity: string | null;
    /** The journal filter's inclusive range; null ends are open. */
    from: string | null;
    to: string | null;
    /** The browser's local today: where an open end stops. */
    today: string;
    /** The `/api/journal` generation the answer must describe. */
    generation: string;
}

export const runningSeries = createResource<RunningSeriesQuery, BalanceSeries>(async (serverUrl, query) =>
    decodeBalanceSeries(await new LedgelineApi(serverUrl).journalRunningSeries(query))
);
