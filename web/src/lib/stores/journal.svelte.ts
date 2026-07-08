// Journal store (WP-03): Svelte 5 runes state holding the normalized journal,
// plus the filtered $derived views consumed by the journal route. `refresh()`
// is called on startup (once settings.serverUrl is set) and by WP-08's poller;
// state is swapped only after a successful normalize, so old data stays
// visible on error (the route shows an error toast from `status`/`error`).

import {HledgerApi} from "$lib/api/client";
import {normalizePrices, normalizeTransactions} from "$lib/api/normalize";
import type {MixedAmount} from "$lib/domain/money";
import type {PriceDirective, Transaction} from "$lib/domain/types";
import {filteredTotals, filterTxns, sortTxnsDesc} from "$lib/journal/rowModel";
import {filters} from "$lib/stores/filters.svelte";
import {settings} from "$lib/stores/settings.svelte";

type JournalStatus = "idle" | "loading" | "ready" | "error";

let txns = $state<Transaction[]>([]);
let accountNames = $state<string[]>([]);
let prices = $state<PriceDirective[]>([]);
let status = $state<JournalStatus>("idle");
let error = $state<string | null>(null);
let fetchedAt = $state<number | null>(null);

let inFlight: Promise<void> | null = null;

/** Cheap change fingerprint (txn count + last tindex) so polling refreshes don't churn every $derived. */
function sameFingerprint(a: readonly Transaction[], b: readonly Transaction[]): boolean {
    return a.length === b.length && (a.length === 0 || a[a.length - 1].index === b[b.length - 1].index);
}

async function doRefresh(): Promise<void> {
    const baseUrl = settings.serverUrl;
    if (baseUrl === null) {
        status = "error";
        error = "No hledger-web server configured";
        return;
    }
    status = "loading";
    try {
        const api = new HledgerApi(baseUrl);
        const [rawTxns, nextNames, rawPrices] = await Promise.all([api.transactions(), api.accountNames(), api.prices()]);
        const nextTxns = normalizeTransactions(rawTxns);
        const nextPrices = normalizePrices(rawPrices);
        if (fetchedAt === null || !sameFingerprint(txns, nextTxns)) {
            txns = nextTxns;
            accountNames = nextNames;
            prices = nextPrices;
        }
        fetchedAt = Date.now();
        status = "ready";
        error = null;
    } catch (cause) {
        status = "error";
        error = cause instanceof Error ? cause.message : String(cause);
    }
}

export const journal = {
    /** Normalized, frozen transactions in journal order. */
    get txns(): Transaction[] {
        return txns;
    },
    get accountNames(): string[] {
        return accountNames;
    },
    get prices(): PriceDirective[] {
        return prices;
    },
    get status(): JournalStatus {
        return status;
    },
    get error(): string | null {
        return error;
    },
    get fetchedAt(): number | null {
        return fetchedAt;
    },
    /** Full refetch of /transactions, /accountnames, /prices. Concurrent calls share one in-flight request. */
    refresh(): Promise<void> {
        inFlight ??= doRefresh().finally(() => {
            inFlight = null;
        });
        return inFlight;
    },
};

// Filtered views (WP-03 contract): pure derivation logic lives in
// lib/journal/rowModel.ts; these wrappers just wire it to the runes graph.
const filtered = $derived.by(() => filterTxns(txns, filters.value));
const filteredSorted = $derived.by(() => sortTxnsDesc(filtered));
const totals = $derived.by(() => filteredTotals(filtered, filters.value.accounts));

/** Transactions matching the current filters, sorted for display (date desc, index desc). */
export function getFilteredTxns(): Transaction[] {
    return filteredSorted;
}

/** Sum of postings in the selected accounts (all postings when none selected) within the filtered txns. */
export function getFilteredTotals(): MixedAmount {
    return totals;
}
