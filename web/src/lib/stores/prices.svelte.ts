// Stock price updates (TODO.md "Stocks"): the Holdings tab's "Update prices"
// button — which currently-held symbols need a quote, whether a first
// `prices.journal` needs creating, and fetching + appending quotes.
//
// Shaped like `budgetStore.svelte.ts`'s `listing`/`createFile` pair: one
// `createResource` for the read, a `busy` flag and a classified-failure
// dispatcher for each write, and — like it — an `afterWrite` that re-reads
// everything the write invalidated. See that method for why the Holdings
// report cannot refresh itself here.
//
// Lives in `$lib/stores/`, not `$lib/holdings/`: that directory's purity test
// (`holdings/purity.test.ts`) forbids anything but relative imports, because it
// is the pure engine WP-10 ported to Rust — a store that calls the API and
// holds Svelte runes state belongs beside `stores/holdings.svelte.ts`, the
// scope store for the same tab, not inside the engine it drives.

import {classify, type EditFailure} from "$lib/api/editFailure";
import {LedgelineApi} from "$lib/api/native";
import {decodeCreatedPricesFile, decodePricesStatus, decodePricesUpdateResponse} from "$lib/api/nativeDecode";
import type {CreatedPricesFile, PricesStatus, PricesUpdateResponse} from "$lib/holdings/pricesTypes";
import {holdingsData, holdingsScope, otherHoldingsData} from "./holdings.svelte";
import {createResource} from "./resource.svelte";
import {settings} from "./settings.svelte";

const status = createResource<string, PricesStatus>(async (serverUrl) => decodePricesStatus(await new LedgelineApi(serverUrl).getPricesStatus()));

let busy = $state(false);
/** The last status key, so revisiting the tab after a load does not re-read it. */
let statusKey: string | null = null;

export type CreateFileOutcome = {ok: true; created: CreatedPricesFile} | {ok: false; failure: EditFailure};
export type UpdateOutcome = {ok: true; result: PricesUpdateResponse} | {ok: false; failure: EditFailure};

export const pricesStore = {
    /** Which symbols need a quote and where a fetch could go. */
    get status() {
        return status;
    },
    get busy(): boolean {
        return busy;
    },

    /** Load the status once per (server, reconnect), and never twice for the same one. */
    async ensureStatus(serverUrl: string, nonce: number): Promise<void> {
        const key = `${nonce}|${serverUrl}`;
        if (key === statusKey) return;
        statusKey = key;
        await this.reloadStatus(serverUrl);
    },

    /** Re-read the status unconditionally (after a write, a Retry, or a global Refresh). */
    async reloadStatus(serverUrl: string): Promise<void> {
        await status.load(serverUrl, serverUrl);
    },

    /**
     * Re-read everything a price write invalidates.
     *
     * Both the status and the Holdings report, always, and awaited — the same
     * claim `budgetStore.afterWrite` makes, for the same reason: the market
     * values are the whole point of pressing this button, and returning while
     * the table still shows the prices from before the fetch is how a screen
     * comes to disagree with itself.
     *
     * `+page.svelte`'s own load effect cannot do this. It is keyed on
     * `(url, nonce, scope)` and a price write moves none of the three, so the
     * report would sit on the pre-update prices until the user changed the
     * scope. The scope here comes from `holdingsScope` — the very store that
     * key reads — so this reload can never be for a scope other than the one on
     * screen.
     *
     * The Other tab is reloaded only once it has actually been opened: before
     * that it holds nothing that could be stale, and plans/14's whole point is
     * that a user who never clicks it pays for nothing.
     */
    async afterWrite(serverUrl: string): Promise<void> {
        await Promise.all([
            this.reloadStatus(serverUrl),
            holdingsData.load(serverUrl, holdingsScope.value),
            otherHoldingsData.report === null ? Promise.resolve() : otherHoldingsData.load(serverUrl, holdingsScope.value),
        ]);
    },

    /** Create a `prices.journal` and include it from the main journal. */
    async createFile(): Promise<CreateFileOutcome> {
        const url = settings.serverUrl;
        if (url === null) return {ok: false, failure: {kind: "unavailable", message: "No server is configured."}};
        busy = true;
        try {
            const created = decodeCreatedPricesFile(await new LedgelineApi(url).createPricesFile());
            await this.reloadStatus(url);
            return {ok: true, created};
        } catch (error) {
            return {ok: false, failure: classify(error)};
        } finally {
            busy = false;
        }
    },

    /** Fetch every currently-held symbol's latest close and append it to `journalId`. */
    async update(journalId: string): Promise<UpdateOutcome> {
        const url = settings.serverUrl;
        if (url === null) return {ok: false, failure: {kind: "unavailable", message: "No server is configured."}};
        busy = true;
        try {
            const result = decodePricesUpdateResponse(await new LedgelineApi(url).updatePrices(journalId));
            await this.afterWrite(url);
            return {ok: true, result};
        } catch (error) {
            return {ok: false, failure: classify(error)};
        } finally {
            busy = false;
        }
    },
};
