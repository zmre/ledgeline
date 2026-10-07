// The Holdings pie's classifications (asset class, sector, …) for the holdings
// on screen: `GET /api/holdings/profiles`.
//
// Fetched only when a category view is actually chosen — "By holding" needs
// nothing from here — and then once per holdings report: `ensure` is keyed on
// the report's own rows (by reference) plus the server, so a scope change, a
// global Refresh or a price update, each of which produces a new report, asks
// again, and re-rendering the same report never does. Asking again is cheap:
// the engine answers from its own cache (`commodity-profiles.json`) and only
// goes to Yahoo for what that cache lacks.
//
// Lives in `$lib/stores/`, not `$lib/holdings/`, for the reason `prices.svelte.ts`
// gives: it calls the API and holds runes state, and `lib/holdings/` is pure.

import {LedgelineApi} from "$lib/api/native";
import {decodeHoldingsProfiles} from "$lib/api/nativeDecode";
import type {HoldingsProfiles} from "$lib/holdings/profileTypes";
import type {Holding} from "$lib/holdings/types";
import {createResource} from "./resource.svelte";

const resource = createResource<readonly string[], HoldingsProfiles>(async (serverUrl, symbols) =>
    decodeHoldingsProfiles(await new LedgelineApi(serverUrl).getHoldingsProfiles(symbols))
);

/** What the last `ensure` asked for, so the same report is never asked about twice. */
let asked: {serverUrl: string; nonce: number; holdings: readonly Holding[]} | null = null;

export const holdingsProfiles = {
    /** The last successful answer, or null before one. May describe an earlier report while a newer one loads. */
    get value(): HoldingsProfiles | null {
        return resource.value;
    },
    get status() {
        return resource.status;
    },
    get error(): Error | null {
        return resource.error;
    },

    /** Load classifications for `holdings`, unless this exact report was already asked about. */
    async ensure(serverUrl: string, nonce: number, holdings: readonly Holding[]): Promise<void> {
        if (asked !== null && asked.serverUrl === serverUrl && asked.nonce === nonce && asked.holdings === holdings) return;
        asked = {serverUrl, nonce, holdings};
        const symbols = holdings.map((holding) => holding.symbol).filter((symbol, i, all) => all.indexOf(symbol) === i);
        await resource.load(serverUrl, symbols.sort());
    },

    /** Ask again for `holdings` regardless (the Retry button). */
    async reload(serverUrl: string, nonce: number, holdings: readonly Holding[]): Promise<void> {
        asked = null;
        await this.ensure(serverUrl, nonce, holdings);
    },
};
